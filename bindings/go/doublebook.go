// Package doublebook calls the Rust doublebook engine through its C ABI.
//
// Build the static library first (cargo build --release in the repository
// root). Point the linker elsewhere with CGO_LDFLAGS when vendoring a
// prebuilt libdoublebook.a, e.g. one cross-compiled for linux/arm64.
package doublebook

/*
#cgo CFLAGS: -I${SRCDIR}/../../include
#cgo LDFLAGS: -L${SRCDIR}/../../target/release -ldoublebook
#cgo linux LDFLAGS: -lm -ldl -lpthread
#cgo darwin LDFLAGS: -liconv
#include <stdlib.h>
#include "doublebook.h"
*/
import "C"

import (
	"encoding/json"
	"fmt"
	"unsafe"
)

// Error is a failed request: Kind is invalid, notFound, draftInScope or badRequest.
type Error struct {
	Kind     string          `json:"kind"`
	Problems []Problem       `json:"problems,omitempty"`
	What     string          `json:"what,omitempty"`
	ID       string          `json:"id,omitempty"`
	EntryIDs []string        `json:"entryIds,omitempty"`
	Message  string          `json:"message,omitempty"`
	Raw      json.RawMessage `json:"-"`
}

// Problem is one broken rule.
type Problem struct {
	Code    string `json:"code"`
	Message string `json:"message"`
	EntryID string `json:"entryId,omitempty"`
	Leg     *int   `json:"leg,omitempty"`
}

func (e *Error) Error() string {
	switch e.Kind {
	case "invalid":
		return fmt.Sprintf("doublebook: invalid (%d problems): %s", len(e.Problems), e.firstProblem())
	case "notFound":
		return fmt.Sprintf("doublebook: %s not found: %s", e.What, e.ID)
	case "draftInScope":
		return fmt.Sprintf("doublebook: %d draft entries in scope", len(e.EntryIDs))
	}
	return "doublebook: " + e.Kind + ": " + e.Message
}

func (e *Error) firstProblem() string {
	if len(e.Problems) == 0 {
		return ""
	}
	return e.Problems[0].Code + ": " + e.Problems[0].Message
}

// Raw sends one request ({"op": ..., ...}) and returns the raw response JSON.
func Raw(request []byte) []byte {
	creq := C.CString(string(request))
	defer C.free(unsafe.Pointer(creq))
	cres := C.doublebook_call(creq)
	defer C.doublebook_free(cres)
	return []byte(C.GoString(cres))
}

// Call marshals request, runs it and unmarshals the result into out (which
// may be nil). A refused request returns *Error.
func Call(request any, out any) error {
	body, err := json.Marshal(request)
	if err != nil {
		return err
	}
	var res struct {
		OK     bool            `json:"ok"`
		Result json.RawMessage `json:"result"`
		Error  json.RawMessage `json:"error"`
	}
	if err := json.Unmarshal(Raw(body), &res); err != nil {
		return err
	}
	if !res.OK {
		e := &Error{Raw: res.Error}
		if err := json.Unmarshal(res.Error, e); err != nil {
			return err
		}
		return e
	}
	if out == nil {
		return nil
	}
	return json.Unmarshal(res.Result, out)
}

// Version is the linked engine's version.
func Version() string { return C.GoString(C.doublebook_version()) }
