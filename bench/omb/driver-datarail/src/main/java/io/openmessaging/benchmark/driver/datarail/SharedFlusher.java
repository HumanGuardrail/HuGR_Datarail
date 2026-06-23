package io.openmessaging.benchmark.driver.datarail;

import java.util.List;
import java.util.concurrent.CopyOnWriteArrayList;

/**
 * One process-wide flusher thread for ALL producers, instead of one thread per producer. At high topic counts
 * a thread-per-producer flusher is a big chunk of the thread oversubscription that caps aggregate throughput on
 * a busy box (measured: per-stream rate fell from 65k at 8 topics to 38k at 16 as threads piled up). A single
 * shared flusher coalesces each producer's buffered writes every ~0.8 ms with one thread total.
 */
final class SharedFlusher {
    private static final List<DatarailBenchmarkProducer> PRODUCERS = new CopyOnWriteArrayList<>();
    private static volatile boolean started;

    private SharedFlusher() {}

    static void register(DatarailBenchmarkProducer p) {
        PRODUCERS.add(p);
        ensureStarted();
    }

    static void unregister(DatarailBenchmarkProducer p) {
        PRODUCERS.remove(p);
    }

    private static synchronized void ensureStarted() {
        if (started) {
            return;
        }
        started = true;
        Thread t = new Thread(SharedFlusher::loop, "datarail-shared-flush");
        t.setDaemon(true);
        t.start();
    }

    private static void loop() {
        try {
            while (true) {
                Thread.sleep(0, 800_000); // ~0.8 ms
                for (DatarailBenchmarkProducer p : PRODUCERS) {
                    p.flushBuffered();
                }
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
    }
}
