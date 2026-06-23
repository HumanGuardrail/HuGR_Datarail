package io.openmessaging.benchmark.driver.datarail;

import io.openmessaging.benchmark.driver.BenchmarkConsumer;
import io.openmessaging.benchmark.driver.ConsumerCallback;
import io.openmessaging.benchmark.driver.datarail.DatarailSelectorLoop.ReadHandler;
import java.io.IOException;
import java.net.InetSocketAddress;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.channels.SelectionKey;
import java.nio.channels.SocketChannel;
import java.nio.charset.StandardCharsets;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * Consumer over the OMB-PROTOCOL egress: after sending its {@code [topic][sub]} header, the shim STREAMS
 * delivered {@code [u32 len][u64 publish_ts_millis][payload]} frames; this hands each whole frame to
 * {@link ConsumerCallback#messageReceived}.
 *
 * <p><b>Threading.</b> No per-consumer reader thread. The socket is a non-blocking {@link SocketChannel}
 * registered with the shared {@link SharedSelectors#CONSUMER_LOOP}, which drives frame parsing for ALL
 * consumers from one thread. All parse state ({@link #acc}, {@link #expectedFrameLen}) is touched only on that
 * loop thread, so no locking is needed in the hot path.
 *
 * <p><b>Partial reads.</b> TCP gives no message framing — a read may end mid-header, mid-payload, or carry
 * several frames at once. {@link #acc} accumulates raw bytes; {@link #parse} emits every COMPLETE frame present
 * (12-byte header + {@code len} payload) and leaves the partial tail buffered for the next wakeup. So a frame
 * is delivered to the callback exactly once, never torn and never duplicated.
 */
public class DatarailBenchmarkConsumer implements BenchmarkConsumer, ReadHandler {
    private static final Logger log = LoggerFactory.getLogger(DatarailBenchmarkConsumer.class);

    /** Per-frame header: u32 payload_len + u64 publish_ts_millis (payload follows). */
    private static final int FRAME_HEADER_BYTES = 12;
    /** Initial accumulation capacity; grown on demand to fit an oversized frame. */
    private static final int INITIAL_ACC = 1 << 16;
    /**
     * Parse-safety ceiling on a single frame's payload. The shim caps a RECORD (ts+payload) at
     * {@code max_record_bytes} (1 MiB default), so a legitimate payload is ≤ that; we allow generous headroom
     * (16 MiB) so we never false-reject a real frame, while still rejecting a garbage/desynced length (which
     * shows up as a huge or negative u32) instead of allocating wildly.
     */
    private static final int MAX_FRAME_PAYLOAD = 16 << 20;

    private final SocketChannel channel;
    private final ConsumerCallback cb;
    private volatile boolean closed;

    // ---- parse state: touched ONLY on the CONSUMER_LOOP thread ----
    // `acc` is in FILL mode between calls (position = bytes accumulated, ready to receive more from read()).
    private ByteBuffer acc = ByteBuffer.allocate(INITIAL_ACC).order(ByteOrder.BIG_ENDIAN);
    // -1 = header not yet parsed; otherwise the payload length of the frame currently being assembled.
    private int expectedFrameLen = -1;
    private long expectedTs;

    DatarailBenchmarkConsumer(String host, int port, String topic, String subscription, ConsumerCallback cb)
            throws IOException {
        this.cb = cb;

        SocketChannel ch = SocketChannel.open();
        try {
            ch.socket().setTcpNoDelay(true);
            ch.configureBlocking(true); // connect + header write blocking, like the old Socket ctor
            ch.connect(new InetSocketAddress(host, port));

            // Header once: [u16 topic_len][topic_utf8][u16 sub_len][sub_utf8]. Blocking, before going non-blocking.
            byte[] t = topic.getBytes(StandardCharsets.UTF_8);
            byte[] s = subscription.getBytes(StandardCharsets.UTF_8);
            ByteBuffer hdr = ByteBuffer.allocate(2 + t.length + 2 + s.length).order(ByteOrder.BIG_ENDIAN);
            hdr.putShort((short) t.length).put(t).putShort((short) s.length).put(s).flip();
            while (hdr.hasRemaining()) {
                ch.write(hdr);
            }

            ch.configureBlocking(false);
        } catch (IOException e) {
            ch.close();
            throw e;
        }
        this.channel = ch;

        SharedSelectors.CONSUMER_LOOP.register(channel, this);
    }

    // ===== frame reader (ReadHandler) — runs ONLY on the CONSUMER_LOOP thread =====

    /** The consumer socket is readable: drain all available bytes and deliver every complete frame. */
    @Override
    public void onReadable(SelectionKey selKey) throws IOException {
        int read;
        // Drain fully: one OP_READ wakeup can have more bytes than `acc` currently holds free.
        while ((read = channel.read(acc)) > 0) {
            parse();
        }
        if (read < 0) {
            throw new IOException("egress closed (EOF) by shim");
        }
    }

    /**
     * Consume every COMPLETE frame currently buffered in {@code acc}, calling the callback once per frame, and
     * leave any partial-frame tail buffered. {@code acc} enters and exits in FILL mode.
     */
    private void parse() throws IOException {
        acc.flip(); // -> read/parse mode: position=0, limit=bytesAccumulated
        while (true) {
            if (expectedFrameLen < 0) {
                // Need a full 12-byte header before we know the frame size.
                if (acc.remaining() < FRAME_HEADER_BYTES) {
                    break;
                }
                int len = acc.getInt();
                long ts = acc.getLong();
                if (len < 0 || len > MAX_FRAME_PAYLOAD) {
                    throw new IOException("egress frame length out of range: " + len);
                }
                expectedFrameLen = len;
                expectedTs = ts;
                ensureCapacityForFrame(len); // header consumed; make sure the rest can buffer if it's huge
            }
            if (acc.remaining() < expectedFrameLen) {
                break; // header parsed, payload not fully arrived yet
            }
            // Whole payload present: hand exactly these `len` bytes to the callback.
            byte[] payload = new byte[expectedFrameLen];
            acc.get(payload);
            cb.messageReceived(payload, expectedTs);
            expectedFrameLen = -1; // ready for the next frame's header
        }
        acc.compact(); // discard consumed bytes, keep the partial tail, -> FILL mode for the next read()
    }

    /**
     * Ensure {@code acc} can eventually hold the whole {@code payloadLen}-byte payload of the frame now being
     * assembled. Called with {@code acc} in PARSE mode and the 12-byte header just consumed, so the unconsumed
     * tail ({@code acc.remaining()}) is the start of that payload. After the parse loop breaks, {@code compact()}
     * moves that tail to offset 0 and later {@code read()}s append the rest — so the full payload fits iff
     * {@code capacity() >= payloadLen}. We size to exactly that (a power of two). Frames are capped at
     * {@link #MAX_FRAME_PAYLOAD}, so capacity stays bounded.
     */
    private void ensureCapacityForFrame(int payloadLen) {
        if (acc.capacity() >= payloadLen) {
            return;
        }
        int newCap = acc.capacity();
        while (newCap < payloadLen) {
            newCap <<= 1;
        }
        // Copy the unconsumed remainder (position..limit, the partial payload) to the front of a bigger buffer,
        // leaving it in PARSE mode (position=0, limit=tailLen) so the surrounding loop's checks still hold.
        ByteBuffer bigger = ByteBuffer.allocate(newCap).order(ByteOrder.BIG_ENDIAN);
        bigger.put(acc); // copies acc[position..limit] (the tail); acc.position advances to limit, harmless
        bigger.flip();
        acc = bigger;
    }

    /** The consumer loop dropped this channel (EOF/error). Nothing to fail; OMB stops the run. Loop thread. */
    @Override
    public void onClosed(Throwable cause) {
        if (!closed) {
            log.debug("consumer stream ended", cause);
        }
    }

    @Override
    public void close() throws Exception {
        closed = true;
        try {
            channel.close(); // cancels the selector key on the CONSUMER_LOOP
        } catch (IOException e) {
            log.debug("close", e);
        }
    }
}
