package doublebook

import (
	"encoding/json"
	"errors"
	"testing"
)

func TestRoundTrip(t *testing.T) {
	if Version() == "" {
		t.Fatal("empty version")
	}
	var book map[string]any
	err := Call(map[string]any{
		"op": "newBook", "nameEnglish": "Test", "nameKorean": "테스트", "closingMonth": 12,
		"baseCurrency": map[string]any{"code": "KRW", "nameEnglish": "Korean won", "nameKorean": "원"},
	}, &book)
	if err != nil {
		t.Fatal(err)
	}
	var bs struct {
		TotalAssets string `json:"totalAssets"`
	}
	if err := Call(map[string]any{"op": "balanceSheet", "book": book, "at": "2026-12-31T23:59", "requireFinal": true}, &bs); err != nil {
		t.Fatal(err)
	}
	if bs.TotalAssets != "0" {
		t.Fatalf("totalAssets = %q", bs.TotalAssets)
	}
	err = Call(map[string]any{"op": "createEntry", "book": book, "entry": json.RawMessage(`{"uniqueId":"00000000-0000-0000-0000-000000000000","time":"2026-01-01T00:00","legs":[],"type":"normal","state":"final"}`)}, nil)
	var e *Error
	if !errors.As(err, &e) || e.Kind != "invalid" || len(e.Problems) == 0 {
		t.Fatalf("want invalid, got %v", err)
	}
}
