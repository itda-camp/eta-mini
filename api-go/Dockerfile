# delivery-eta Go 서비스 이미지. 저장소 루트를 빌드 컨텍스트로 쓴다.
#   docker build -f api-go/Dockerfile -t eta-go .
#   docker run --rm -p 8081:8080 eta-go

# ── ONNX Runtime: 버전은 uv.lock 의 onnxruntime 을 따른다 (Python 서빙과 같은 런타임) ──
FROM debian:trixie-slim AS onnxruntime
ARG TARGETARCH
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
COPY uv.lock /tmp/uv.lock
RUN set -eu; \
    ORT_VERSION=$(awk '/^name = "onnxruntime"$/ {found=1; next} found && /^version = / {gsub(/"/, "", $3); print $3; exit}' /tmp/uv.lock); \
    [ -n "$ORT_VERSION" ] || { echo "uv.lock 에서 onnxruntime 버전을 찾지 못했다" >&2; exit 1; }; \
    case "$TARGETARCH" in \
      amd64) ORT_ARCH=x64 ;; \
      arm64) ORT_ARCH=aarch64 ;; \
      *) echo "지원하지 않는 TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    echo "onnxruntime ${ORT_VERSION} (linux-${ORT_ARCH})"; \
    curl -fsSL "https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/onnxruntime-linux-${ORT_ARCH}-${ORT_VERSION}.tgz" \
      | tar -xz -C /opt; \
    mv "/opt/onnxruntime-linux-${ORT_ARCH}-${ORT_VERSION}" /opt/onnxruntime

# ── Go 빌드: onnxruntime_go 는 cgo 로 libonnxruntime 을 dlopen 한다 ──
FROM golang:1.27-trixie AS build
WORKDIR /src
COPY api-go/go.mod api-go/go.sum ./
RUN go mod download
COPY api-go/*.go ./
RUN CGO_ENABLED=1 go build -trimpath -ldflags="-s -w" -o /out/eta-go .

# ── 실행 이미지 ──
FROM debian:trixie-slim
RUN useradd --system --no-create-home app
COPY --from=onnxruntime /opt/onnxruntime/lib/ /opt/onnxruntime/lib/
COPY --from=build /out/eta-go /usr/local/bin/eta-go
COPY artifacts/model_sklearn.onnx /app/model_sklearn.onnx
ENV ONNXRUNTIME_LIB=/opt/onnxruntime/lib/libonnxruntime.so \
    MODEL_PATH=/app/model_sklearn.onnx \
    PORT=8080
USER app
EXPOSE 8080
CMD ["eta-go"]
