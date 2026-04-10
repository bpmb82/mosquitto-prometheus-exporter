# Mosquitto Prometheus Exporter

A lightweight Prometheus exporter written in Rust that monitors Mosquitto broker `$SYS` metrics and exposes them via an HTTP endpoint.

## Features

* **Custom Metrics**: Easily configure which topics to monitor via a YAML configuration file.
* **Robust Parsing**: Uses Regex to extract numerical values from complex payloads (e.g., extracting `154544` from `"154544 seconds"`).
* **TLS Support**: Built-in support for secure connections using the `tls://` prefix.
* **Clean Output**: Specifically designed to exclude standard HTTP server metrics, providing only the MQTT data you care about.
* **Efficient**: Built on the Tokio runtime for high performance and low resource consumption.

## Installation

Ensure you have the Rust toolchain installed.

```bash
cargo build --release
```

## Configuration

The application looks for a configuration file at /config.yml. If this file is missing, the exporter will fall back to a comprehensive set of built-in defaults.
Example config.yml
```
metrics:
  - topic: "$SYS/broker/uptime"
    name: "mqtt_uptime_seconds"
    help: "Total uptime of the broker in seconds"
  - topic: "$SYS/broker/clients/connected"
    name: "mqtt_clients_connected"
    help: "Number of currently connected clients"
  - topic: "$SYS/broker/load/bytes/received/1min"
    name: "mqtt_load_bytes_received_1min"
    help: "Average bytes received per minute over the last minute"
  - topic: "$SYS/broker/messages/sent"
    name: "mqtt_messages_sent_total"
    help: "Total number of messages sent since startup"
```

### Environment Variables

You can configure the exporter via a .env file or system environment variables:

| Variable | Default | Description |
| :--- | :--- | :--- |
| MQTT_HOST | tcp://127.0.0.1 | Broker address (use tls:// for SSL/TLS) |
| MQTT_PORT | 1883 | Network port of the MQTT broker |
| MQTT_USER | - | MQTT Username (optional) |
| MQTT_PASS | - | Password (optional) |
| HTTP_PORT | 9090 | Port to serve Prometheus metrics on |
| LOGLEVEL | info | Logging verbosity (error, warn, info, debug) |

## Usage

### CLI

Run the compiled binary:
```
./mosquitto-prometheus-exporter
```

Access your metrics at:
http://localhost:9090/metrics

### Docker

Run the docker image:
```
docker run -ti -e "MQTT_HOST=tls://mosquitto.remotehome.cloud" -e "MQTT_PORT=8883" -p "9090:9090" bpmbee/mosquitto-exporter:latest
```

## Prometheus Integration

Add the following job to your prometheus.yml configuration:
```
scrape_configs:
  - job_name: 'mosquitto_exporter'
    static_configs:
      - targets: ['localhost:9090']
```
## License

MIT / Apache-2.0