/* doublebook C ABI. Link libdoublebook.a (cargo build --release). */
#ifndef DOUBLEBOOK_H
#define DOUBLEBOOK_H

#ifdef __cplusplus
extern "C" {
#endif

/* Handles one JSON request ({"op": ..., ...}) and returns a JSON response:
 * {"ok": true, "result": ...} or {"ok": false, "error": {"kind": ..., ...}}.
 * Free the result with doublebook_free. Thread-safe: no global state. */
char *doublebook_call(const char *request);

void doublebook_free(char *response);

/* Static string; do not free. */
const char *doublebook_version(void);

#ifdef __cplusplus
}
#endif

#endif
