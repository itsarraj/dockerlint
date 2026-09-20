FROM golang:1.22 AS builder
WORKDIR /src
COPY . .
RUN go build -o /out/app .

FROM ubuntu:22.04
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /out/app /usr/local/bin/app
ADD https://example.com/data.tar.gz /opt/data.tar.gz
HEALTHCHECK --interval=30s CMD app -health || exit 1
USER appuser
CMD ["app"]
