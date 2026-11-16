package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"math"
	"math/big"
	"mime"
	"net/http"
	"strconv"
	"strings"
)

// PredictFunc 는 (거리 km, 대기 주문 수) → 예상 시간(분) 이다.
type PredictFunc func(distanceKm, pendingOrders float64) (float64, error)

const maxBodyBytes = 1 << 20

// fieldError 는 FastAPI(pydantic) 422 응답의 detail 항목과 같은 모양이다.
type fieldError struct {
	Type  string `json:"type"`
	Loc   []any  `json:"loc"`
	Msg   string `json:"msg"`
	Input any    `json:"input"`
}

// NewHandler 는 api/app.py 와 같은 계약의 라우터를 만든다.
func NewHandler(predict PredictFunc, model string) http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("/health", func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet {
			methodNotAllowed(w, http.MethodGet)
			return
		}
		writeJSON(w, http.StatusOK, map[string]string{"status": "ok", "model": model})
	})
	mux.HandleFunc("/eta", func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost {
			methodNotAllowed(w, http.MethodPost)
			return
		}
		handleEta(w, r, predict, model)
	})
	mux.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		writeJSON(w, http.StatusNotFound, map[string]string{"detail": "Not Found"})
	})
	return mux
}

func handleEta(w http.ResponseWriter, r *http.Request, predict PredictFunc, model string) {
	raw, err := io.ReadAll(http.MaxBytesReader(w, r.Body, maxBodyBytes))
	if err != nil {
		var tooLarge *http.MaxBytesError
		if errors.As(err, &tooLarge) {
			writeJSON(w, http.StatusRequestEntityTooLarge, map[string]string{"detail": "Request body too large"})
			return
		}
		writeJSON(w, http.StatusBadRequest, map[string]string{"detail": "요청 본문을 읽지 못했습니다."})
		return
	}

	distance, orders, errs := parseEtaRequest(r.Header.Get("Content-Type"), raw)
	if len(errs) > 0 {
		writeJSON(w, http.StatusUnprocessableEntity, map[string]any{"detail": errs})
		return
	}

	minutes, err := predict(distance, orders)
	if err != nil {
		writeJSON(w, http.StatusInternalServerError, map[string]string{"detail": "예측에 실패했습니다."})
		return
	}
	if math.IsNaN(minutes) || math.IsInf(minutes, 0) {
		// 입력은 유효하지만 너무 커서 계산이 넘친 경우 (예: distance_km=1e308)
		writeJSON(w, http.StatusUnprocessableEntity, map[string]string{"detail": "입력이 너무 커서 예상 시간을 계산할 수 없습니다."})
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"minutes": minutes, "model": model})
}

// parseEtaRequest 는 EtaRequest 검증 규칙을 그대로 따른다.
//   - distance_km: JSON 숫자(정수·소수)만, 유한, >= 0. 문자열·bool·null 은 거부.
//   - pending_orders: JSON 정수 표기만(2.0, 1e2 거부), >= 0. 아주 큰 정수도 받는다.
//   - 모르는 필드는 무시한다.
func parseEtaRequest(contentType string, raw []byte) (float64, float64, []fieldError) {
	if contentType != "" && !isJSONMediaType(contentType) {
		return 0, 0, []fieldError{{Type: "model_attributes_type", Loc: []any{"body"},
			Msg: "Input should be a valid dictionary or object to extract fields from", Input: string(raw)}}
	}
	if len(bytes.TrimSpace(raw)) == 0 {
		return 0, 0, []fieldError{{Type: "missing", Loc: []any{"body"}, Msg: "Field required", Input: nil}}
	}

	// Python json 처럼 NaN·Infinity 리터럴을 받아 들인 뒤 필드 검증에서 거부한다(깨진 JSON 이 아니라 필드 오류).
	dec := json.NewDecoder(bytes.NewReader(markNonFinite(raw)))
	dec.UseNumber()
	var body any
	if err := dec.Decode(&body); err != nil {
		return 0, 0, []fieldError{jsonInvalid(err)}
	}
	if _, err := dec.Token(); err != io.EOF {
		return 0, 0, []fieldError{{Type: "json_invalid", Loc: []any{"body", dec.InputOffset()}, Msg: "JSON decode error", Input: map[string]any{}}}
	}

	obj, ok := body.(map[string]any)
	if !ok {
		return 0, 0, []fieldError{{Type: "model_attributes_type", Loc: []any{"body"},
			Msg: "Input should be a valid dictionary or object to extract fields from", Input: restoreNonFinite(body)}}
	}

	var errs []fieldError
	distance, e := parseDistance(obj)
	if e != nil {
		errs = append(errs, *e)
	}
	orders, e := parseOrders(obj)
	if e != nil {
		errs = append(errs, *e)
	}
	return distance, orders, errs
}

func parseDistance(obj map[string]any) (float64, *fieldError) {
	const name = "distance_km"
	v, present := obj[name]
	loc := []any{"body", name}
	if !present {
		return 0, &fieldError{"missing", loc, "Field required", restoreNonFinite(obj)}
	}
	switch x := v.(type) {
	case json.Number:
		f, err := strconv.ParseFloat(string(x), 64)
		if err != nil || math.IsInf(f, 0) { // 1e400 처럼 float64 를 넘는 값
			return 0, &fieldError{"finite_number", loc, "Input should be a finite number", x}
		}
		if f < 0 {
			return 0, &fieldError{"greater_than_equal", loc, "Input should be greater than or equal to 0", x}
		}
		return f, nil
	case string:
		if lit, ok := nonFiniteLiteral(x); ok {
			return 0, &fieldError{"finite_number", loc, "Input should be a finite number", lit}
		}
	}
	return 0, &fieldError{"float_type", loc, "Input should be a valid number", restoreNonFinite(v)}
}

func parseOrders(obj map[string]any) (float64, *fieldError) {
	const name = "pending_orders"
	v, present := obj[name]
	loc := []any{"body", name}
	if !present {
		return 0, &fieldError{"missing", loc, "Field required", restoreNonFinite(obj)}
	}
	x, ok := v.(json.Number)
	if !ok || strings.ContainsAny(string(x), ".eE") {
		return 0, &fieldError{"int_type", loc, "Input should be a valid integer", restoreNonFinite(v)}
	}
	n, ok := new(big.Int).SetString(string(x), 10)
	if !ok {
		return 0, &fieldError{"int_type", loc, "Input should be a valid integer", x}
	}
	if n.Sign() < 0 {
		return 0, &fieldError{"greater_than_equal", loc, "Input should be greater than or equal to 0", x}
	}
	f, _ := new(big.Float).SetInt(n).Float64() // 아주 큰 정수는 +Inf → 예측이 넘쳐 422
	return f, nil
}

func isJSONMediaType(contentType string) bool {
	mt, _, err := mime.ParseMediaType(contentType)
	if err != nil {
		return false
	}
	return mt == "application/json" || (strings.HasPrefix(mt, "application/") && strings.HasSuffix(mt, "+json"))
}

func jsonInvalid(err error) fieldError {
	var offset int64
	var syn *json.SyntaxError
	if errors.As(err, &syn) {
		offset = syn.Offset
	}
	return fieldError{Type: "json_invalid", Loc: []any{"body", offset}, Msg: "JSON decode error", Input: map[string]any{}}
}

// 문자열 바깥의 NaN·Infinity·-Infinity 리터럴을 이 표식이 붙은 JSON 문자열로 바꾼다.
const nonFiniteMark = "\x00nonfinite:"

var nonFiniteTokens = []string{"-Infinity", "Infinity", "NaN"}

func markNonFinite(raw []byte) []byte {
	var out bytes.Buffer
	inString, escaped := false, false
	for i := 0; i < len(raw); {
		c := raw[i]
		if inString {
			out.WriteByte(c)
			switch {
			case escaped:
				escaped = false
			case c == '\\':
				escaped = true
			case c == '"':
				inString = false
			}
			i++
			continue
		}
		if c == '"' {
			inString = true
			out.WriteByte(c)
			i++
			continue
		}
		replaced := false
		for _, tok := range nonFiniteTokens {
			if bytes.HasPrefix(raw[i:], []byte(tok)) {
				out.WriteString(`"\u0000nonfinite:` + tok + `"`)
				i += len(tok)
				replaced = true
				break
			}
		}
		if !replaced {
			out.WriteByte(c)
			i++
		}
	}
	return out.Bytes()
}

// nonFiniteLiteral 은 표식이 붙은 문자열이면 Python str(float) 표기("nan", "inf", "-inf")를 돌려준다.
func nonFiniteLiteral(s string) (string, bool) {
	lit, ok := strings.CutPrefix(s, nonFiniteMark)
	if !ok {
		return "", false
	}
	return map[string]string{"NaN": "nan", "Infinity": "inf", "-Infinity": "-inf"}[lit], true
}

// restoreNonFinite 는 오류 응답의 input 에 실을 값에서 표식을 원래 리터럴 문자열로 되돌린다.
func restoreNonFinite(v any) any {
	switch x := v.(type) {
	case string:
		if lit, ok := nonFiniteLiteral(x); ok {
			return lit
		}
	case map[string]any:
		m := make(map[string]any, len(x))
		for k, val := range x {
			m[k] = restoreNonFinite(val)
		}
		return m
	case []any:
		s := make([]any, len(x))
		for i, val := range x {
			s[i] = restoreNonFinite(val)
		}
		return s
	}
	return v
}

func methodNotAllowed(w http.ResponseWriter, allow string) {
	w.Header().Set("Allow", allow)
	writeJSON(w, http.StatusMethodNotAllowed, map[string]string{"detail": "Method Not Allowed"})
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(v)
}
