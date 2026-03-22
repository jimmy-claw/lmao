/* Stub implementations for RLN symbols to allow linking without librln.
   These functions are referenced by libwaku's Nim-compiled RLN relay code
   but are never called when RLN is not actively used. */
#include <stddef.h>
#include <stdint.h>

void* new(size_t depth, const void* config) { return NULL; }
int set_metadata(void* ctx, const void* input) { return 0; }
int get_metadata(void* ctx, void* output) { return 0; }
int flush(void* ctx) { return 0; }
int delete_leaf(void* ctx, size_t index) { return 0; }
int get_root(void* ctx, void* output) { return 0; }
int generate_rln_proof(void* ctx, const void* input, void* output) { return 0; }
int verify_with_roots(void* ctx, const void* proof, const void* roots, size_t roots_len) { return 0; }
int atomic_operation(void* ctx, size_t index, const void* input, void* output) { return 0; }
void* poseidon_hash(const void* input, size_t len, void* output) { return NULL; }
