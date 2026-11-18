# delivery-eta Go 서비스 이미지. 저장소 루트를 빌드 컨텍스트로 쓴다.
#   docker build -f api-go/Dockerfile -t eta-go .
#   docker run --rm -p 8081:8080 eta-go

# ── ONNX Runtime: 버전은 uv.lock 의 onnxruntime 을 따른다 (Python 서빙과 같은 런타임) ──
FROM debian:trixie-slim AS onnxruntime
ARG TARGETARCH
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl unzip \
    && rm -rf /var/lib/apt/lists/*
COPY uv.lock /tmp/uv.lock
RUN set -eu; \
    ORT_VERSION=$(awk '/^name = "onnxruntime"$/ {found=1; next} found && /^version = / {gsub(/"/, "", $3); print $3; exit}' /tmp/uv.lock); \
    [ -n "$ORT_VERSION" ] || { echo "uv.lock 에서 onnxruntime 버전을 찾지 못했다" >&2; exit 1; }; \
    case "$TARGETARCH" in \
      amd64) ORT_RID=linux-x64 ;; \
      arm64) ORT_RID=linux-arm64 ;; \
      *) echo "지원하지 않는 TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    echo "onnxruntime ${ORT_VERSION} (${ORT_RID})"; \
    # GitHub 릴리스 대신 NuGet 패키지에서 받는다 — 같은 libonnxruntime.so 가 runtimes/<플랫폼>/native/ 에 들어 있다
    curl -fsSL "https://www.nuget.org/api/v2/package/Microsoft.ML.OnnxRuntime/${ORT_VERSION}" -o /tmp/ort.nupkg; \
    mkdir -p /opt/onnxruntime/lib; \
    unzip -j -q /tmp/ort.nupkg "runtimes/${ORT_RID}/native/*" -d /opt/onnxruntime/lib; \
    rm /tmp/ort.nupkg

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
