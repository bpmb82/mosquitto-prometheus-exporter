FROM scratch

ARG TARGETARCH

COPY bin-${TARGETARCH}/mosquitto-prometheus-exporter /mosquitto-prometheus-exporter

EXPOSE 9090

ENTRYPOINT ["/mosquitto-prometheus-exporter"]