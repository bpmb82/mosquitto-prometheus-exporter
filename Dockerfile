FROM scratch

ARG TARGETARCH

COPY --chmod=755 bin-${TARGETARCH}/mosquitto-prometheus-exporter /mosquitto-prometheus-exporter

EXPOSE 9090

ENTRYPOINT ["/mosquitto-prometheus-exporter"]