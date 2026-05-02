use actix_web::{get, web, App, HttpResponse, HttpServer, Responder};
use anyhow::{Context, Result};
use prometheus::{Encoder, Gauge, Registry, TextEncoder};
use regex::Regex;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS, Transport};
use std::collections::HashMap;
use std::{env, sync::Arc};
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

struct MetricsState {
    registry: Registry,
    gauges: RwLock<HashMap<String, Gauge>>,
}

#[get("/")]
async fn metrics_handler(state: web::Data<Arc<MetricsState>>) -> impl Responder {
    let encoder = TextEncoder::new();
    let mut buffer = Vec::new();
    let metric_families = state.registry.gather();

    if let Err(e) = encoder.encode(&metric_families, &mut buffer) {
        error!("Could not encode prometheus metrics: {}", e);
        return HttpResponse::InternalServerError().body("Error encoding metrics");
    }

    HttpResponse::Ok()
        .content_type("text/plain")
        .body(buffer)
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));
    
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .init();

    let mqtt_host = env::var("MQTT_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let mqtt_port = env::var("MQTT_PORT").unwrap_or_else(|_| "1883".to_string()).parse::<u16>()?;
    let mqtt_tls = env::var("MQTT_TLS").unwrap_or_else(|_| "false".to_string()).parse::<bool>().unwrap_or(false);
    let http_port = env::var("HTTP_PORT").unwrap_or_else(|_| "9090".to_string()).parse::<u16>()?;

    let mut mqttoptions = MqttOptions::new("mosquitto-exporter", &mqtt_host, mqtt_port);
    mqttoptions.set_keep_alive(Duration::from_secs(5));

    if mqtt_tls {
        info!("Configuring TLS for MQTT connection to {}:{}", mqtt_host, mqtt_port);
        use rumqttc::tokio_rustls::rustls;
        let mut root_store = rustls::RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        
        let tls_config = rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        mqttoptions.set_transport(Transport::tls_with_config(tls_config.into()));
    } else {
        info!("Connecting to MQTT broker at {}:{} via TCP", mqtt_host, mqtt_port);
    }

    if let (Ok(user), Ok(pass)) = (env::var("MQTT_USER"), env::var("MQTT_PASS")) {
        mqttoptions.set_credentials(user, pass);
    }

    let state = Arc::new(MetricsState {
        registry: Registry::new(),
        gauges: RwLock::new(HashMap::new()),
    });

    let (client, mut eventloop) = AsyncClient::new(mqttoptions, 10);
    let state_for_task = state.clone();
    let re = Regex::new(r"(\d+\.?\d*)")?;

    tokio::spawn(async move {
        loop {
            match eventloop.poll().await {
                Ok(notification) => {
                    match notification {
                        Event::Incoming(Packet::ConnAck(_)) => {
                            info!("Successfully connected to MQTT broker");
                            if let Err(e) = client.subscribe("$SYS/#", QoS::AtMostOnce).await {
                                error!("Failed to subscribe to $SYS/#: {}", e);
                            } else {
                                info!("Subscribed to $SYS/# topics");
                            }
                        }
                        Event::Incoming(Packet::Publish(publish)) => {
                            let topic = publish.topic.clone();
                            let payload = String::from_utf8_lossy(&publish.payload);

                            if let Some(caps) = re.captures(&payload) {
                                if let Ok(val) = caps[1].parse::<f64>() {
                                    let gauges_read = state_for_task.gauges.read().await;
                                    if let Some(gauge) = gauges_read.get(&topic) {
                                        gauge.set(val);
                                    } else {
                                        drop(gauges_read);
                                        let mut gauges_write = state_for_task.gauges.write().await;
                                        if !gauges_write.contains_key(&topic) {
                                            let base_name = topic.replace("$SYS", "mqtt");
                                            let metric_name = base_name
                                                .chars()
                                                .map(|c| if c.is_ascii_alphanumeric() || c == ':' { c } else { '_' })
                                                .collect::<String>();

                                            match Gauge::new(&metric_name, format!("Metric for {}", topic)) {
                                                Ok(gauge) => {
                                                    if let Err(e) = state_for_task.registry.register(Box::new(gauge.clone())) {
                                                        warn!("Failed to register metric {}: {}", metric_name, e);
                                                    } else {
                                                        gauge.set(val);
                                                        gauges_write.insert(topic, gauge);
                                                    }
                                                }
                                                Err(e) => error!("Invalid metric name {}: {}", metric_name, e),
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Err(e) => {
                    error!("MQTT error: {}. Retrying in 5s...", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    });

    info!("Starting Prometheus exporter webserver on 0.0.0.0:{}", http_port);
    
    let state_data = web::Data::new(state);
    HttpServer::new(move || {
        App::new()
            .app_data(state_data.clone())
            .service(metrics_handler)
    })
    .bind(("0.0.0.0", http_port))?
    .run()
    .await
    .context("Server crash")
}