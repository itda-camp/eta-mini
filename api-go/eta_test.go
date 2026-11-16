package main

import (
	"encoding/json"
	"math"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// rulePredict 는 api/predictors/rule.py 와 같은 식이다. 핸들러 계약만 확인하려고 모델 대신 쓴다.
// float32 를 거치는 ONNX 모델처럼 float32 범위를 넘으면 +Inf 가 된다.
func rulePredict(d, o float64) (float64, error) {
	return float64(float32(10.0 + 3.0*d + 2.0*o)), nil
}

func do(t *testing.T, method, path, body, contentType string) (*httptest.ResponseRecorder, map[string]any) {
	t.Helper()
	req := httptest.NewRequest(method, path, strings.NewReader(body))
	if contentType != "" {
		req.Header.Set("Content-Type", contentType)
	}
	rec := httptest.NewRecorder()
	NewHandler(rulePredict, "onnx").ServeHTTP(rec, req)
	var parsed map[string]any
	_ = json.Unmarshal(rec.Body.Bytes(), &parsed)
	return rec, parsed
}

func postJSON(t *testing.T, body string) (*httptest.ResponseRecorder, map[string]any) {
	return do(t, http.MethodPost, "/eta", body, "application/json")
}

// /health 는 200 과 status=ok, 구현체 이름을 돌려준다.
func TestHealth(t *testing.T) {
	rec, body := do(t, http.MethodGet, "/health", "", "")
	if rec.Code != 200 || len(body) != 2 || body["status"] != "ok" || body["model"] != "onnx" {
		t.Fatalf("got %d %v", rec.Code, body)
	}
}

// /eta 는 정상·경계 입력과 모르는 필드·-0.0·아주 큰 정수에 200 과 예상 분, 구현체 이름을 돌려준다.
func TestEtaKnownInputs(t *testing.T) {
	cases := []struct {
		body string
		want float64
	}{
		{`{"distance_km": 0, "pending_orders": 0}`, 10},
		{`{"distance_km": 2, "pending_orders": 3}`, 22},
		{`{"distance_km": 1.5, "pending_orders": 0}`, 14.5},
		{`{"distance_km": 1000, "pending_orders": 10000}`, 23010},
		{`{"distance_km": 1, "pending_orders": 0, "x": 1}`, 13},
		{`{"distance_km": -0.0, "pending_orders": 0}`, 10},
		{`{"distance_km": 0, "pending_orders": 100000000000000000000000}`, 2e23},
	}
	for _, c := range cases {
		rec, body := postJSON(t, c.body)
		if rec.Code != 200 || len(body) != 2 || body["model"] != "onnx" {
			t.Errorf("%s: got %d %v", c.body, rec.Code, body)
			continue
		}
		if got := body["minutes"].(float64); math.Abs(got-c.want) > 1e-6*math.Max(1, c.want) {
			t.Errorf("%s: minutes=%v want %v", c.body, got, c.want)
		}
	}
}

// /eta 는 음수·숫자 아님·정수 아님·null·문자열 숫자·bool·필드 누락에 422 를 돌려준다.
func TestEtaInvalidFieldsReturn422(t *testing.T) {
	for _, b := range []string{
		`{"distance_km": -1, "pending_orders": 0}`,
		`{"distance_km": 0, "pending_orders": -1}`,
		`{"distance_km": -1e-9, "pending_orders": 0}`,
		`{"distance_km": "abc", "pending_orders": 0}`,
		`{"distance_km": 1, "pending_orders": "many"}`,
		`{"distance_km": 1, "pending_orders": 1.5}`,
		`{"distance_km": 1, "pending_orders": 2.0}`,
		`{"distance_km": 1, "pending_orders": 1e2}`,
		`{"distance_km": "2", "pending_orders": 3}`,
		`{"distance_km": 2, "pending_orders": "3"}`,
		`{"distance_km": true, "pending_orders": 0}`,
		`{"distance_km": 1, "pending_orders": false}`,
		`{"distance_km": null, "pending_orders": 0}`,
		`{"distance_km": 0, "pending_orders": null}`,
		`{"distance_km": 1}`,
		`{"pending_orders": 1}`,
		`{}`,
	} {
		if rec, _ := postJSON(t, b); rec.Code != 422 {
			t.Errorf("%s: got %d, want 422", b, rec.Code)
		}
	}
}

// /eta 는 객체가 아닌 JSON·깨진 JSON·빈 본문·form 인코딩 본문에 422 를 돌려준다.
func TestEtaMalformedBodyReturns422(t *testing.T) {
	cases := [][2]string{
		{"[1, 2]", "application/json"},
		{"{bad", "application/json"},
		{`{"distance_km": 1, "pending_orders": 0} x`, "application/json"},
		{"", "application/json"},
		{"distance_km=1&pending_orders=0", "application/x-www-form-urlencoded"},
	}
	for _, c := range cases {
		if rec, _ := do(t, http.MethodPost, "/eta", c[0], c[1]); rec.Code != 422 {
			t.Errorf("%q (%s): got %d, want 422", c[0], c[1], rec.Code)
		}
	}
}

// /eta 의 422 응답은 detail[].loc 로 문제 필드를 가리킨다 (NaN·Infinity 리터럴 포함).
func TestEta422DetailPointsToField(t *testing.T) {
	for _, b := range []string{
		`{"distance_km": -1, "pending_orders": 0}`,
		`{"distance_km": NaN, "pending_orders": 0}`,
		`{"distance_km": Infinity, "pending_orders": 0}`,
		`{"distance_km": -Infinity, "pending_orders": 0}`,
		`{"distance_km": 1e400, "pending_orders": 0}`,
	} {
		rec, body := postJSON(t, b)
		if rec.Code != 422 {
			t.Errorf("%s: got %d, want 422", b, rec.Code)
			continue
		}
		found := false
		for _, e := range body["detail"].([]any) {
			loc := e.(map[string]any)["loc"].([]any)
			if len(loc) == 2 && loc[0] == "body" && loc[1] == "distance_km" {
				found = true
			}
		}
		if !found {
			t.Errorf("%s: detail 에 [body, distance_km] 가 없다: %v", b, body)
		}
	}
}

// /eta 는 계산이 넘칠 만큼 큰 거리(1e308)에 minutes=null 대신 422 를 돌려준다.
func TestEtaOverflowReturns422(t *testing.T) {
	rec, body := postJSON(t, `{"distance_km": 1e308, "pending_orders": 0}`)
	if rec.Code != 422 {
		t.Fatalf("got %d %v", rec.Code, body)
	}
	if _, ok := body["detail"].(string); !ok {
		t.Fatalf("detail 이 문자열이 아니다: %v", body)
	}
}

// /eta 는 GET 에, /health 는 POST 에 405 를 돌려준다.
func TestMethodNotAllowed(t *testing.T) {
	if rec, _ := do(t, http.MethodGet, "/eta", "", ""); rec.Code != 405 {
		t.Errorf("GET /eta: got %d", rec.Code)
	}
	if rec, _ := do(t, http.MethodPost, "/health", "", ""); rec.Code != 405 {
		t.Errorf("POST /health: got %d", rec.Code)
	}
}
