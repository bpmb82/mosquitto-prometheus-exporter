FROM scratch

ARG TARGETARCH

COPY bin-${TARGETARCH}/mosquitto-prometheus-exporter /mosquitto-prometheus-exporter

ENTRYPOINT ["/mosquitto-prometheus-exporter"]