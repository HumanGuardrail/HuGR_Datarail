package io.openmessaging.benchmark.driver.datarail;

import com.fasterxml.jackson.databind.DeserializationFeature;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.dataformat.yaml.YAMLFactory;
import io.openmessaging.benchmark.driver.BenchmarkConsumer;
import io.openmessaging.benchmark.driver.BenchmarkDriver;
import io.openmessaging.benchmark.driver.BenchmarkProducer;
import io.openmessaging.benchmark.driver.ConsumerCallback;
import java.io.File;
import java.io.IOException;
import java.io.UncheckedIOException;
import java.util.concurrent.CompletableFuture;
import org.apache.bookkeeper.stats.StatsLogger;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * OpenMessaging-Benchmark driver for datarail. Bridges OMB's pub/sub onto datarail's point-to-point sealed rail
 * via the {@code datarail-omb-shim} (see docs/design/OMB-PROTOCOL.md). This is a benchmark adapter, NOT a
 * product component — it maps topics/subscriptions onto the shim's ingress/egress sockets so the REAL OMB
 * harness measures datarail through the same interface it uses for Kafka/Pulsar/RabbitMQ.
 */
public class DatarailBenchmarkDriver implements BenchmarkDriver {
    private static final Logger log = LoggerFactory.getLogger(DatarailBenchmarkDriver.class);

    private String ingressHost;
    private int ingressPort;
    private String egressHost;
    private int egressPort;

    @Override
    public void initialize(File configurationFile, StatsLogger statsLogger) throws IOException {
        ObjectMapper mapper = new ObjectMapper(new YAMLFactory());
        mapper.configure(DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES, false);
        DatarailConfig cfg = mapper.readValue(configurationFile, DatarailConfig.class);
        String[] ing = splitHostPort(cfg.ingressAddr);
        String[] egr = splitHostPort(cfg.egressAddr);
        this.ingressHost = ing[0];
        this.ingressPort = Integer.parseInt(ing[1]);
        this.egressHost = egr[0];
        this.egressPort = Integer.parseInt(egr[1]);
        log.info("datarail driver: ingress {}:{} egress {}:{}", ingressHost, ingressPort, egressHost, egressPort);
    }

    private static String[] splitHostPort(String addr) {
        int i = addr.lastIndexOf(':');
        if (i < 0) {
            throw new IllegalArgumentException("expected host:port, got " + addr);
        }
        return new String[] {addr.substring(0, i), addr.substring(i + 1)};
    }

    @Override
    public String getTopicNamePrefix() {
        return "datarail-omb";
    }

    @Override
    public CompletableFuture<Void> createTopic(String topic, int partitions) {
        // Topics are lazy in the shim (created on first producer/consumer reference).
        return CompletableFuture.completedFuture(null);
    }

    @Override
    public CompletableFuture<BenchmarkProducer> createProducer(String topic) {
        return CompletableFuture.supplyAsync(
                () -> {
                    try {
                        return new DatarailBenchmarkProducer(ingressHost, ingressPort, topic);
                    } catch (IOException e) {
                        throw new UncheckedIOException(e);
                    }
                });
    }

    @Override
    public CompletableFuture<BenchmarkConsumer> createConsumer(
            String topic, String subscriptionName, ConsumerCallback consumerCallback) {
        return CompletableFuture.supplyAsync(
                () -> {
                    try {
                        return new DatarailBenchmarkConsumer(
                                egressHost, egressPort, topic, subscriptionName, consumerCallback);
                    } catch (IOException e) {
                        throw new UncheckedIOException(e);
                    }
                });
    }

    @Override
    public void close() {
        // Producers/consumers own their sockets and close themselves.
    }
}
