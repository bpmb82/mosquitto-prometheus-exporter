use actix_web::{get, web, App, HttpResponse, HttpServer, Responder};
use anyhow::{Context, Result};
use prometheus::{Encoder, Gauge, Registry, TextEncoder};
use regex::Regex;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS, Transport};
use serde::Deserialize;
use std::collections::HashMap;
use std::{env, fs};
use std::sync::Arc;
use std::time::Duration;
use tracing::{error};

#[derive(Debug, Deserialize, Clone)]
struct MetricConfig {
    topic: String,
    name: String,
    help: String,
}

#[derive(Debug, Deserialize)]
struct Config {
    metrics: Vec<MetricConfig>,
}

fn get_default_metrics() -> Vec<MetricConfig> {
    vec![
        MetricConfig {
            topic: "$SYS/broker/uptime".into(),
            name: "mqtt_uptime_seconds".into(),
            help: "Broker uptime".into(),
        },
        MetricConfig {
            topic: "$SYS/broker/bytes/sent".into(),
            name: "mqtt_bytes_sent_total".into(),
            help: "Total bytes sent".into(),
        },
        MetricConfig {
            topic: "$SYS/broker/bytes/received".into(),
            name: "mqtt_bytes_received_total".into(),
            help: "Total bytes received".into(),
        },
        MetricConfig {
            topic: "$SYS/broker/messages/sent".into(),
            name: "mqtt_messages_sent_total".into(),
            help: "Total messages sent".into(),
        },
        MetricConfig {
            topic: "$SYS/broker/messages/received".into(),
            name: "mqtt_messages_received_total".into(),
            help: "Total messages received".into(),
        },
        MetricConfig {
            topic: "$SYS/broker/load/bytes/sent/1min".into(),
            name: "mqtt_load_bytes_sent_1min".into(),
            help: "Average bytes sent per minute".into(),
        },
        MetricConfig {
            topic: "$SYS/broker/load/bytes/received/1min".into(),
            name: "mqtt_load_bytes_received_1min".into(),
            help: "Average bytes received per minute".into(),
        },
        MetricConfig {
            topic: "$SYS/broker/connections/socket/count".into(),
            name: "mqtt_socket_count".into(),
            help: "Current socket count".into(),
        },
    ]
}

#[get("/")]
async fn metrics_handler(registry: web::Data<Registry>) -> impl Responder {
    let encoder = TextEncoder::new();
    let mut buffer = Vec::new();
    let metric_families = registry.gather();

    // Vervang .unwrap() door een match of result-afhandeling
    if let Err(e) = encoder.encode(&metric_families, &mut buffer) {
        error!("Could not encode prometheus metrics: {}", e);
        return HttpResponse::InternalServerError()
            .body("Error encoding metrics");
    }

    HttpResponse::Ok()
        .content_type("text/plain")
        .body(buffer)
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    let log_level = env::var("LOGLEVEL").unwrap_or_else(|_| "info".into());
    tracing_subscriber::fmt().with_env_filter(log_level).init();

    let config_path = "/config.yml";
    let metrics_configs = if let Ok(content) = fs::read_to_string(config_path) {
        let cfg: Config = serde_yaml::from_str(&content).context("Failed to parse config.yml")?;
        cfg.metrics
    } else {
        get_default_metrics()
    };

    let mqtt_url = env::var("MQTT_HOST").unwrap_or_else(|_| "tcp://127.0.0.1".to_string());
    let mqtt_port = env::var("MQTT_PORT").unwrap_or_else(|_| "1883".to_string()).parse::<u16>()?;
    let http_port = env::var("HTTP_PORT").unwrap_or_else(|_| "9090".to_string()).parse::<u16>()?;

    let (is_tls, host) = if mqtt_url.starts_with("tls://") {
        (true, mqtt_url.replace("tls://", ""))
    } else {
        (false, mqtt_url.replace("tcp://", ""))
    };

    let mut mqttoptions = MqttOptions::new("mosquitto-exporter", &host, mqtt_port);
    mqttoptions.set_keep_alive(Duration::from_secs(5));

    if is_tls {
        use rumqttc::tokio_rustls::rustls;
        let mut root_store = rustls::RootCertStore::empty();
        root_store.extend(
            webpki_roots::TLS_SERVER_ROOTS
                .iter()
                .cloned()
        );
        
        let tls_config = rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        mqttoptions.set_transport(Transport::tls_with_config(tls_config.into()));
    }

    if let (Ok(user), Ok(pass)) = (env::var("MQTT_USER"), env::var("MQTT_PASS")) {
        mqttoptions.set_credentials(user, pass);
    }

    let registry = Registry::new();
    let mut metrics_map = HashMap::new();

    for cfg in metrics_configs {
        let gauge = Gauge::new(&cfg.name, &cfg.help)?;
        registry.register(Box::new(gauge.clone()))?;
        metrics_map.insert(cfg.topic.clone(), gauge);
    }

    let registry = Arc::new(registry);
    let (client, mut eventloop) = AsyncClient::new(mqttoptions, 10);
    for topic in metrics_map.keys() {
        client.subscribe(topic, QoS::AtMostOnce).await?;
    }

    let metrics_for_task = metrics_map.clone();
    let re = Regex::new(r"(\d+\.?\d*)")?;

    tokio::spawn(async move {
        loop {
            match eventloop.poll().await {
                Ok(notification) => {
                    if let Event::Incoming(Packet::Publish(publish)) = notification {
                        if let Some(gauge) = metrics_for_task.get(&publish.topic) {
                            let payload = String::from_utf8_lossy(&publish.payload);
                            if let Some(caps) = re.captures(&payload) {
                                if let Ok(val) = caps[1].parse::<f64>() {
                                    gauge.set(val);
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("MQTT error: {}. Retrying...", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    });

    let registry_data = web::Data::from(registry);

    HttpServer::new(move || {
        App::new()
            .app_data(registry_data.clone())
            .service(metrics_handler)
    })
    .workers(2)
    .bind(("0.0.0.0", http_port))?
    .run()
    .await
    .context("Server crash")
}