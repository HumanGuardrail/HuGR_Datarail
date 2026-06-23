package io.openmessaging.benchmark.driver.datarail;

import java.io.IOException;
import java.nio.channels.SelectionKey;
import java.nio.channels.Selector;
import java.nio.channels.SocketChannel;
import java.util.Iterator;
import java.util.Set;
import java.util.concurrent.ConcurrentLinkedQueue;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * One shared NIO {@link Selector} + one daemon thread that services MANY non-blocking sockets, replacing the
 * old thread-per-connection model (1 ack-reader thread per producer + 1 reader thread per consumer ⇒ ~2N
 * threads at N streams). With this loop, N connections cost ONE thread, killing the thread oversubscription
 * that capped aggregate throughput on a busy box.
 *
 * <p>Each registered channel carries a {@link ReadHandler} as its {@link SelectionKey} attachment. When the
 * channel is readable the loop calls {@link ReadHandler#onReadable(SelectionKey)} ON THE SELECTOR THREAD; the
 * handler drains everything available and parses whole protocol units (acks / frames). All per-connection
 * parse state lives in the handler, so a single-threaded loop needs no locking to parse.
 *
 * <p><b>Concurrent registration.</b> OMB creates producers/consumers concurrently from its own threads, but a
 * {@link Selector} and its key set are NOT safe to mutate from another thread while {@code select()} is in
 * flight (registration can block on the selector's internal lock until the next select returns). We use the
 * standard wakeup + registration-queue pattern: {@link #register} enqueues the request and calls
 * {@link Selector#wakeup()}; the loop drains the queue and performs every {@link SocketChannel#register} on its
 * OWN thread before the next {@code select()}. Deregistration (close) just cancels the key, which IS
 * thread-safe, and wakes the loop so the cancelled key is reaped promptly.
 */
final class DatarailSelectorLoop {
    private static final Logger log = LoggerFactory.getLogger(DatarailSelectorLoop.class);

    /** Handles read-readiness for one channel. Invoked only on the loop thread; holds the channel's parse state. */
    interface ReadHandler {
        /**
         * The channel is readable. Drain all currently-available bytes and process every complete protocol unit.
         *
         * @throws IOException on a fatal socket/protocol error; the loop will cancel the key and close the channel.
         */
        void onReadable(SelectionKey key) throws IOException;

        /** The channel left the loop (EOF, error, or close). Called once on the loop thread for cleanup/teardown. */
        void onClosed(Throwable cause);
    }

    private static final class Registration {
        final SocketChannel channel;
        final ReadHandler handler;

        Registration(SocketChannel channel, ReadHandler handler) {
            this.channel = channel;
            this.handler = handler;
        }
    }

    private final String name;
    private final Selector selector;
    private final Thread thread;
    private final ConcurrentLinkedQueue<Registration> pendingRegistrations = new ConcurrentLinkedQueue<>();
    private volatile boolean running = true;

    DatarailSelectorLoop(String name) {
        this.name = name;
        try {
            this.selector = Selector.open();
        } catch (IOException e) {
            throw new IllegalStateException("cannot open selector for " + name, e);
        }
        this.thread = new Thread(this::loop, name);
        this.thread.setDaemon(true);
        this.thread.start();
    }

    /**
     * Register a non-blocking channel for OP_READ with its handler. Safe to call from any thread: the actual
     * {@link SocketChannel#register} happens on the loop thread, so it never races {@code select()}.
     */
    void register(SocketChannel channel, ReadHandler handler) {
        pendingRegistrations.add(new Registration(channel, handler));
        selector.wakeup();
    }

    private void loop() {
        try {
            while (running) {
                // Apply queued registrations on THIS thread before selecting, so we never mutate the key set
                // concurrently with another thread's select().
                drainRegistrations();

                int n = selector.select(); // blocks until a key is ready or wakeup() (registration/close)
                if (!running) {
                    break;
                }
                if (n == 0) {
                    continue; // woken for a registration/cancel only
                }

                Set<SelectionKey> keys = selector.selectedKeys();
                Iterator<SelectionKey> it = keys.iterator();
                while (it.hasNext()) {
                    SelectionKey key = it.next();
                    it.remove();
                    if (!key.isValid()) {
                        continue;
                    }
                    if (key.isReadable()) {
                        ReadHandler handler = (ReadHandler) key.attachment();
                        try {
                            handler.onReadable(key);
                        } catch (IOException | RuntimeException e) {
                            // Fatal for THIS connection only: cancel + close, notify the handler, keep the loop.
                            closeKey(key, handler, e);
                        }
                    }
                }
            }
        } catch (Throwable t) {
            // A failure here would silently freeze every stream on this loop, so make it loud.
            log.error("{}: selector loop terminated unexpectedly", name, t);
        } finally {
            shutdownSelector();
        }
    }

    private void drainRegistrations() {
        Registration r;
        while ((r = pendingRegistrations.poll()) != null) {
            try {
                r.channel.register(selector, SelectionKey.OP_READ, r.handler);
            } catch (Throwable e) {
                // Registration failed (e.g. channel already closed): tell the handler so its futures don't hang.
                r.handler.onClosed(e);
                try {
                    r.channel.close();
                } catch (IOException ignored) {
                    // already gone
                }
            }
        }
    }

    private static void closeKey(SelectionKey key, ReadHandler handler, Throwable cause) {
        key.cancel();
        try {
            key.channel().close();
        } catch (IOException ignored) {
            // best effort
        }
        handler.onClosed(cause);
    }

    private void shutdownSelector() {
        for (SelectionKey key : selector.keys()) {
            try {
                key.channel().close();
            } catch (IOException ignored) {
                // best effort
            }
        }
        try {
            selector.close();
        } catch (IOException ignored) {
            // best effort
        }
    }

    /** Stop the loop and close the selector. Not used in the OMB run (daemon thread dies with the JVM). */
    void shutdown() {
        running = false;
        selector.wakeup();
    }
}
