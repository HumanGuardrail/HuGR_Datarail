package io.openmessaging.benchmark.driver.datarail;

import io.openmessaging.benchmark.driver.BenchmarkProducer;
import java.io.BufferedInputStream;
import java.io.BufferedOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.net.Socket;
import java.util.Optional;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentLinkedQueue;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * Producer over the OMB-PROTOCOL ingress: writes {@code [u32 len][u64 publish_ts_millis][payload]} frames and
 * completes each {@link #sendAsync} future in FIFO order as the shim returns {@code [u64 seq]} acks (≈ acks=1).
 */
public class DatarailBenchmarkProducer implements BenchmarkProducer {
    private static final Logger log = LoggerFactory.getLogger(DatarailBenchmarkProducer.class);

    private final Socket socket;
    private final DataOutputStream out;
    private final DataInputStream in;
    private final ConcurrentLinkedQueue<CompletableFuture<Void>> pending = new ConcurrentLinkedQueue<>();
    private final Thread ackReader;
    private volatile boolean closed;

    DatarailBenchmarkProducer(String host, int port, String topic) throws IOException {
        this.socket = new Socket(host, port);
        this.socket.setTcpNoDelay(true);
        this.out = new DataOutputStream(new BufferedOutputStream(socket.getOutputStream(), 1 << 16));
        this.in = new DataInputStream(new BufferedInputStream(socket.getInputStream(), 1 << 16));
        byte[] t = topic.getBytes(java.nio.charset.StandardCharsets.UTF_8);
        synchronized (out) {
            out.writeShort(t.length);
            out.write(t);
            out.flush();
        }
        this.ackReader = new Thread(this::readAcks, "datarail-ack-" + topic);
        this.ackReader.setDaemon(true);
        this.ackReader.start();
        // sendAsync writes into the buffered stream WITHOUT flushing per message (a per-message flush is one
        // syscall per message — it caps producer throughput). A single process-wide SharedFlusher coalesces
        // every producer's buffer every ~0.8 ms, bounding latency without a thread per producer.
        SharedFlusher.register(this);
    }

    /** Flush this producer's buffered writes; called by the shared flusher. Swallows errors after close. */
    void flushBuffered() {
        try {
            synchronized (out) {
                out.flush();
            }
        } catch (IOException e) {
            if (!closed) {
                log.debug("flush", e);
            }
        }
    }

    private void readAcks() {
        try {
            while (!closed) {
                in.readLong(); // seq — monotonic, FIFO; value unused, order is the contract
                CompletableFuture<Void> f = pending.poll();
                if (f != null) {
                    f.complete(null);
                }
            }
        } catch (IOException e) {
            if (!closed) {
                failAll(e);
            }
        }
    }

    private void failAll(Throwable t) {
        CompletableFuture<Void> f;
        while ((f = pending.poll()) != null) {
            f.completeExceptionally(t);
        }
    }

    @Override
    public CompletableFuture<Void> sendAsync(Optional<String> key, byte[] payload) {
        CompletableFuture<Void> future = new CompletableFuture<>();
        try {
            long ts = System.currentTimeMillis();
            synchronized (out) {
                pending.add(future); // enqueue BEFORE write so ack order == enqueue order == receive order
                out.writeInt(payload.length);
                out.writeLong(ts);
                out.write(payload);
                // No per-message flush: the background flusher (~0.8 ms) coalesces writes into few syscalls.
            }
        } catch (IOException e) {
            future.completeExceptionally(e);
        }
        return future;
    }

    @Override
    public void close() throws Exception {
        closed = true;
        SharedFlusher.unregister(this);
        try {
            socket.close();
        } catch (IOException e) {
            log.debug("close", e);
        }
    }
}
