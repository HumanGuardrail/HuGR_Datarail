package io.openmessaging.benchmark.driver.datarail;

import io.openmessaging.benchmark.driver.BenchmarkConsumer;
import io.openmessaging.benchmark.driver.ConsumerCallback;
import java.io.BufferedInputStream;
import java.io.BufferedOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.net.Socket;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * Consumer over the OMB-PROTOCOL egress: after sending its {@code [topic][sub]} header, it streams delivered
 * {@code [u32 len][u64 publish_ts_millis][payload]} frames and hands each to {@link ConsumerCallback}.
 */
public class DatarailBenchmarkConsumer implements BenchmarkConsumer {
    private static final Logger log = LoggerFactory.getLogger(DatarailBenchmarkConsumer.class);

    private final Socket socket;
    private final Thread reader;
    private volatile boolean closed;

    DatarailBenchmarkConsumer(String host, int port, String topic, String subscription, ConsumerCallback cb)
            throws IOException {
        this.socket = new Socket(host, port);
        this.socket.setTcpNoDelay(true);
        DataOutputStream out = new DataOutputStream(new BufferedOutputStream(socket.getOutputStream()));
        byte[] t = topic.getBytes(java.nio.charset.StandardCharsets.UTF_8);
        byte[] s = subscription.getBytes(java.nio.charset.StandardCharsets.UTF_8);
        out.writeShort(t.length);
        out.write(t);
        out.writeShort(s.length);
        out.write(s);
        out.flush();
        DataInputStream in = new DataInputStream(new BufferedInputStream(socket.getInputStream(), 1 << 16));
        this.reader = new Thread(() -> readLoop(in, cb), "datarail-consume-" + topic + "-" + subscription);
        this.reader.setDaemon(true);
        this.reader.start();
    }

    private void readLoop(DataInputStream in, ConsumerCallback cb) {
        try {
            while (!closed) {
                int len = in.readInt();
                long ts = in.readLong();
                byte[] payload = new byte[len];
                in.readFully(payload);
                cb.messageReceived(payload, ts);
            }
        } catch (IOException e) {
            if (!closed) {
                log.debug("consumer read loop ended", e);
            }
        }
    }

    @Override
    public void close() throws Exception {
        closed = true;
        try {
            socket.close();
        } catch (IOException e) {
            log.debug("close", e);
        }
    }
}
