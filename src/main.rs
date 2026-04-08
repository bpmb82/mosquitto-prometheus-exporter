use actix_web::{App, HttpServer};
use actix_web_prometheus::PrometheusMetricsBuilder;
use anyhow::{Context, Result};
use prometheus::{Gauge, Registry};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS, Transport};
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    let log_level = env::var("LOGLEVEL").unwrap_or_else(|_| "info".into());
    tracing_subscriber::fmt().with_env_filter(log_level).init();

    info!("Starting Mosquitto Exporter (TLS fixed)...");

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
        info!("TLS detected. Loading native root certificates for: {}", host);
        
        use rumqttc::tokio_rustls::rustls;
        
        let mut root_store = rustls::RootCertStore::empty();
        
        for cert in rustls_native_certs::load_native_certs()
            .context("Could not load platform certificates")? {
            root_store.add(cert)?;
        }

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
    let topics = vec![
        ("$SYS/broker/clients/connected", "mqtt_clients_connected", "Connected clients"),
        ("$SYS/broker/uptime", "mqtt_uptime_seconds", "Broker uptime"),
        ("$SYS/broker/memory/current", "mqtt_memory_bytes", "Memory usage"),
    ];

    for (topic, name, help) in topics {
        let gauge = Gauge::new(name, help)?;
        registry.register(Box::new(gauge.clone()))?;
        metrics_map.insert(topic.to_string(), gauge);
    }

    let (client, mut eventloop) = AsyncClient::new(mqttoptions, 10);
    for topic in metrics_map.keys() {
        client.subscribe(topic, QoS::AtMostOnce).await?;
    }

    let metrics_for_task = metrics_map.clone();
    tokio::spawn(async move {
        loop {
            match eventloop.poll().await {
                Ok(notification) => {
                    if let Event::Incoming(Packet::Publish(publish)) = notification {
                        if let Some(gauge) = metrics_for_task.get(&publish.topic) {
                            let payload = String::from_utf8_lossy(&publish.payload);
                            if let Ok(val) = payload.trim().parse::<f64>() {
                                gauge.set(val);
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

    let prometheus = PrometheusMetricsBuilder::new("mosquitto_exporter")
        .registry(registry)
        .endpoint("/metrics")
        .build()?;

    info!("Metrics server at http://0.0.0.0:{}/metrics", http_port);
    HttpServer::new(move || App::new().wrap(prometheus.clone()))
        .bind(("0.0.0.0", http_port))?
        .run()
        .await
        .context("Server crash")
}