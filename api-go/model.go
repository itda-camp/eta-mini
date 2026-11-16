package main

import (
	"fmt"

	ort "github.com/yalue/onnxruntime_go"
)

// OnnxModel 은 입력 "X" [1, 2] float32 → 출력 "variable" [1, 1] float32 모델을 실행한다.
// 정규화는 모델 안에 있으므로 원래 단위 값을 그대로 넣는다.
// DynamicAdvancedSession 은 Run 마다 텐서를 받으므로 여러 요청이 동시에 불러도 된다.
type OnnxModel struct {
	session *ort.DynamicAdvancedSession
}

// LoadOnnxModel 은 ONNX Runtime 공유 라이브러리를 초기화하고 모델 세션을 연다.
func LoadOnnxModel(libPath, modelPath string) (*OnnxModel, error) {
	ort.SetSharedLibraryPath(libPath)
	if err := ort.InitializeEnvironment(); err != nil {
		return nil, fmt.Errorf("ONNX Runtime 초기화 실패 (%s): %w", libPath, err)
	}
	session, err := ort.NewDynamicAdvancedSession(modelPath, []string{"X"}, []string{"variable"}, nil)
	if err != nil {
		_ = ort.DestroyEnvironment()
		return nil, fmt.Errorf("모델 세션 생성 실패 (%s): %w", modelPath, err)
	}
	return &OnnxModel{session: session}, nil
}

// Predict 는 배달 예상 시간(분)을 돌려준다. float32 로 넘치는 입력은 Inf/NaN 결과가 되고,
// 이는 핸들러가 422 로 바꾼다.
func (m *OnnxModel) Predict(distanceKm, pendingOrders float64) (float64, error) {
	input, err := ort.NewTensor(ort.NewShape(1, 2), []float32{float32(distanceKm), float32(pendingOrders)})
	if err != nil {
		return 0, err
	}
	defer input.Destroy()
	output, err := ort.NewEmptyTensor[float32](ort.NewShape(1, 1))
	if err != nil {
		return 0, err
	}
	defer output.Destroy()
	if err := m.session.Run([]ort.Value{input}, []ort.Value{output}); err != nil {
		return 0, err
	}
	return float64(output.GetData()[0]), nil
}

// Close 는 세션과 ONNX Runtime 환경을 정리한다.
func (m *OnnxModel) Close() {
	_ = m.session.Destroy()
	_ = ort.DestroyEnvironment()
}
