// delivery-eta 의 Go 구현. api/app.py 와 같은 HTTP 계약(POST /eta, GET /health)을 따르고,
// 예측은 ONNX 모델(model_sklearn.onnx)을 ONNX Runtime 으로 실행한다.
//
// 환경 변수:
//
//	PORT            듣는 포트 (기본 8080)
//	MODEL_PATH      ONNX 모델 경로 (기본 artifacts/model_sklearn.onnx)
//	ONNXRUNTIME_LIB libonnxruntime 공유 라이브러리 경로 (기본 libonnxruntime.so)
package main

import (
	"context"
	"errors"
	"log"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"
)

// modelName 은 /health 와 /eta 응답의 model 값이다 (Python 의 MODEL_IMPL=onnx 와 같은 이름).
const modelName = "onnx"

func getenv(key, fallback string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return fallback
}

func main() {
	model, err := LoadOnnxModel(
		getenv("ONNXRUNTIME_LIB", "libonnxruntime.so"),
		getenv("MODEL_PATH", "artifacts/model_sklearn.onnx"),
	)
	if err != nil {
		log.Fatalf("모델을 불러오지 못했다: %v", err)
	}
	defer model.Close()

	srv := &http.Server{
		Addr:              ":" + getenv("PORT", "8080"),
		Handler:           NewHandler(model.Predict, modelName),
		ReadHeaderTimeout: 5 * time.Second,
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	go func() {
		<-ctx.Done()
		shutdownCtx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		_ = srv.Shutdown(shutdownCtx)
	}()

	log.Printf("delivery-eta (go, model=%s) listening on %s", modelName, srv.Addr)
	if err := srv.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
		log.Fatal(err)
	}
}
