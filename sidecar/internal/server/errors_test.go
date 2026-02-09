package server

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestWriteJSON(t *testing.T) {
	rec := httptest.NewRecorder()
	writeJSON(rec, http.StatusOK, map[string]string{"key": "value"})

	if rec.Code != http.StatusOK {
		t.Errorf("status = %d, want %d", rec.Code, http.StatusOK)
	}
	if ct := rec.Header().Get("Content-Type"); ct != "application/json" {
		t.Errorf("Content-Type = %q, want %q", ct, "application/json")
	}

	var resp map[string]string
	if err := json.NewDecoder(rec.Body).Decode(&resp); err != nil {
		t.Fatalf("decode: %v", err)
	}
	if resp["key"] != "value" {
		t.Errorf("key = %q, want %q", resp["key"], "value")
	}
}

func TestWriteError(t *testing.T) {
	rec := httptest.NewRecorder()
	writeError(rec, http.StatusBadRequest, "test_code", "test message")

	if rec.Code != http.StatusBadRequest {
		t.Errorf("status = %d, want %d", rec.Code, http.StatusBadRequest)
	}

	var resp ErrorResponse
	if err := json.NewDecoder(rec.Body).Decode(&resp); err != nil {
		t.Fatalf("decode: %v", err)
	}
	if resp.Error != "test message" {
		t.Errorf("error = %q, want %q", resp.Error, "test message")
	}
	if resp.Code != "test_code" {
		t.Errorf("code = %q, want %q", resp.Code, "test_code")
	}
}

func TestErrorResponseShape(t *testing.T) {
	// Verify the JSON shape of error responses matches our contract.
	rec := httptest.NewRecorder()
	writeError(rec, http.StatusInternalServerError, "server_error", "something failed")

	var raw map[string]interface{}
	if err := json.NewDecoder(rec.Body).Decode(&raw); err != nil {
		t.Fatalf("decode: %v", err)
	}

	// Must have "error" field.
	if _, ok := raw["error"]; !ok {
		t.Error("missing 'error' field in response")
	}

	// Must have "code" field.
	if _, ok := raw["code"]; !ok {
		t.Error("missing 'code' field in response")
	}

	// "details" should be omitted when empty (omitempty).
	if _, ok := raw["details"]; ok {
		t.Error("'details' should be omitted when empty")
	}
}
