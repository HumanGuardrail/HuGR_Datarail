package io.openmessaging.benchmark.driver.datarail;

/** Driver config (YAML), per docs/design/OMB-PROTOCOL.md. */
public class DatarailConfig {
    /** host:port the shim accepts producer connections on. */
    public String ingressAddr = "127.0.0.1:7701";
    /** host:port the shim streams delivered messages from. */
    public String egressAddr = "127.0.0.1:7702";
}
