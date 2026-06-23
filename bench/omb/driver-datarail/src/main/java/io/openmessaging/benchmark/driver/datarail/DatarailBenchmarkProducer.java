package io.openmessaging.benchmark.driver.datarail;

import io.openmessaging.benchmark.driver.BenchmarkProducer;
import io.openmessaging.benchmark.driver.datarail.DatarailSelectorLoop.ReadHandler;
import java.io.IOException;
import java.net.InetSocketAddress;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.channels.SelectionKey;
import java.nio.channels.SocketChannel;
import java.nio.charset.StandardCharsets;
import java.util.Optional;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentLinkedQueue;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * Producer over the OMB-PROTOCOL ingress: writes {@code [u32 len][u64 publish_ts_millis][payload]} frames and
 * completes each {@link #sendAsync} future in FIFO order as the shim returns {@code [u64 seq]} acks (≈ acks=1).
 *
 * <p><b>Threading.</b> No per-producer reader thread. The socket is a non-blocking {@link SocketChannel}
 * registered with the shared {@link SharedSelectors#ACK_LOOP}; that one loop drains acks for ALL producers. The
 * send path stays buffered + driven by the single {@link SharedFlusher}: {@link #sendAsync} appends a frame to
 * an in-memory buffer with no syscall (the old design's contract), and the flusher drains that buffer to the
 * channel every ~0.8 ms. So a producer adds ZERO threads (vs. one ack-reader thread before).
 */
public class DatarailBenchmarkProducer implements BenchmarkProducer, ReadHandler {
    private static final Logger log = LoggerFactory.getLogger(DatarailBenchmarkProducer.class);

    /** Bytes per ack on the wire: a single big-endian u64 seq. */
    private static final int ACK_BYTES = 8;
    /** Per-message frame header: u32 payload_len + u64 publish_ts_millis (payload follows). */
    private static final int FRAME_HEADER_BYTES = 12;
    /** Read buffer for draining the ack stream; many acks per wakeup at speed. */
    private static final int ACK_READ_BUF = 1 << 16;

    private final SocketChannel channel;

    // FIFO of outstanding sends. Enqueued under `lock` BEFORE the frame is buffered, so queue order == on-wire
    // order == ack order. Drained head-first by the ack loop. ConcurrentLinkedQueue: producer thread offers,
    // ack-loop thread polls — no shared lock between the two needed for the queue itself.
    private final ConcurrentLinkedQueue<CompletableFuture<Void>> pending = new ConcurrentLinkedQueue<>();

    // ---- send-side buffer (guarded by `lock`) ----
    private final Object lock = new Object();
    private ByteBuffer writeBuf; // accumulation mode (position = write cursor); flipped to drain in flushBuffered
    private boolean closed;

    // ---- ack-side parse state (touched ONLY on the ACK_LOOP thread) ----
    // Held in FILL mode between calls; physically retains the <8-byte tail of a partially-read ack across
    // wakeups (via compact), so the buffer's own state is the only partial-ack bookkeeping needed.
    private final ByteBuffer ackRead = ByteBuffer.allocate(ACK_READ_BUF).order(ByteOrder.BIG_ENDIAN);

    DatarailBenchmarkProducer(String host, int port, String topic) throws IOException {
        this.writeBuf = ByteBuffer.allocate(1 << 16).order(ByteOrder.BIG_ENDIAN);

        SocketChannel ch = SocketChannel.open();
        try {
            ch.socket().setTcpNoDelay(true);
            ch.configureBlocking(true); // connect + header write blocking, like the old Socket ctor
            ch.connect(new InetSocketAddress(host, port));

            // Header once: [u16 topic_len][topic_utf8]. Written blocking before we go non-blocking.
            byte[] t = topic.getBytes(StandardCharsets.UTF_8);
            ByteBuffer hdr = ByteBuffer.allocate(2 + t.length).order(ByteOrder.BIG_ENDIAN);
            hdr.putShort((short) t.length).put(t).flip();
            while (hdr.hasRemaining()) {
                ch.write(hdr);
            }

            ch.configureBlocking(false); // now non-blocking for the shared selector + buffered drains
        } catch (IOException e) {
            ch.close();
            throw e;
        }
        this.channel = ch;

        // Register with the shared ack reader (this == the ReadHandler) and the shared flusher.
        SharedSelectors.ACK_LOOP.register(channel, this);
        SharedFlusher.register(this);
    }

    @Override
    public CompletableFuture<Void> sendAsync(Optional<String> key, byte[] payload) {
        CompletableFuture<Void> future = new CompletableFuture<>();
        long ts = System.currentTimeMillis();
        synchronized (lock) {
            if (closed) {
                future.completeExceptionally(new IOException("producer closed"));
                return future;
            }
            // Enqueue BEFORE buffering the frame so ack order == enqueue order == on-wire order.
            pending.add(future);
            ensureWritable(FRAME_HEADER_BYTES + payload.length);
            writeBuf.putInt(payload.length);
            writeBuf.putLong(ts);
            writeBuf.put(payload);
            // No per-message syscall: the shared flusher (~0.8 ms) coalesces buffered frames into few writes.
        }
        return future;
    }

    /** Grow the accumulation buffer if {@code need} more bytes don't fit. Caller holds {@code lock}. */
    private void ensureWritable(int need) {
        if (writeBuf.remaining() >= need) {
            return;
        }
        int required = writeBuf.position() + need;
        int newCap = writeBuf.capacity();
        while (newCap < required) {
            newCap <<= 1;
        }
        ByteBuffer bigger = ByteBuffer.allocate(newCap).order(ByteOrder.BIG_ENDIAN);
        writeBuf.flip();
        bigger.put(writeBuf);
        writeBuf = bigger; // left in accumulation mode (position = end of copied data)
    }

    /**
     * Drain this producer's buffered frames to the (non-blocking) channel; called by the shared flusher. A
     * non-blocking write may be short when the socket buffer is full — we retain the unwritten remainder for the
     * next flush (the shim drains continuously, so it makes room). Swallows errors after close.
     */
    void flushBuffered() {
        try {
            synchronized (lock) {
                if (closed || writeBuf.position() == 0) {
                    return; // nothing buffered
                }
                writeBuf.flip(); // drain mode: position=0, limit=bytesBuffered
                while (writeBuf.hasRemaining()) {
                    int wrote = channel.write(writeBuf);
                    if (wrote == 0) {
                        break; // socket buffer full; keep remainder, retry next flush
                    }
                }
                writeBuf.compact(); // discard written bytes, keep any remainder, back to accumulation mode
            }
        } catch (IOException e) {
            if (!closed) {
                // The channel is dead; fail outstanding sends so OMB doesn't hang on never-completing futures.
                failAll(e);
                log.debug("flush", e);
            }
        }
    }

    // ===== ack reader (ReadHandler) — runs ONLY on the ACK_LOOP thread =====

    /**
     * The producer socket is readable: drain every byte available and complete one outstanding future per
     * complete 8-byte ack, in FIFO order. Acks arrive in receive order == enqueue order, so head-of-queue is
     * always the message this ack acknowledges; we need only the COUNT of complete acks, not the seq VALUE. A
     * read ending mid-ack leaves a {@code <8}-byte tail that {@code compact()} retains for the next wakeup, so
     * an ack split across reads is completed exactly once, when its 8th byte arrives.
     */
    @Override
    public void onReadable(SelectionKey selKey) throws IOException {
        int read;
        // Drain fully: one OP_READ wakeup may have more bytes queued than ackRead currently holds free.
        while ((read = channel.read(ackRead)) > 0) {
            ackRead.flip(); // -> parse mode: position=0, limit=allUnconsumedBytes (prior tail + just read)
            int complete = ackRead.remaining() / ACK_BYTES; // whole acks present
            ackRead.position(ackRead.position() + complete * ACK_BYTES); // skip them (value unused)
            for (int i = 0; i < complete; i++) {
                CompletableFuture<Void> f = pending.poll();
                if (f != null) {
                    f.complete(null);
                }
            }
            ackRead.compact(); // keep the <8-byte tail at the front, -> FILL mode for the next read()
        }
        if (read < 0) {
            throw new IOException("ingress closed (EOF) by shim");
        }
    }

    /** The ack loop dropped this channel (EOF/error). Fail any still-outstanding sends; called on loop thread. */
    @Override
    public void onClosed(Throwable cause) {
        if (!closed) {
            failAll(cause);
        }
    }

    private void failAll(Throwable t) {
        CompletableFuture<Void> f;
        while ((f = pending.poll()) != null) {
            f.completeExceptionally(t);
        }
    }

    @Override
    public void close() throws Exception {
        synchronized (lock) {
            closed = true;
        }
        SharedFlusher.unregister(this);
        try {
            channel.close(); // cancels the selector key on the ACK_LOOP; onClosed handles any stragglers
        } catch (IOException e) {
            log.debug("close", e);
        }
    }
}
