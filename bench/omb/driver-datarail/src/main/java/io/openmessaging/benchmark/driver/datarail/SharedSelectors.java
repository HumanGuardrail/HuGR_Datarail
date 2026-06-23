package io.openmessaging.benchmark.driver.datarail;

/**
 * Process-wide singletons for the two shared NIO event loops. ALL producer sockets share one ack-reader loop;
 * ALL consumer sockets share one frame-reader loop. So at N streams the driver runs THREE threads total — one
 * ack loop + one consumer loop + the one {@link SharedFlusher} — instead of the old ~2N
 * (one ack-reader + one read-loop thread per connection).
 */
final class SharedSelectors {
    static final DatarailSelectorLoop ACK_LOOP = new DatarailSelectorLoop("datarail-ack-selector");
    static final DatarailSelectorLoop CONSUMER_LOOP = new DatarailSelectorLoop("datarail-consume-selector");

    private SharedSelectors() {}
}
