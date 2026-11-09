/* Thin wrappers over rcheevos' rc_hash so Rust never mirrors its structs. */
#include <string.h>

#include "rc_hash.h"

int heronite_rc_hash_file(char out[33], uint32_t console_id, const char* path)
{
  return rc_hash_generate_from_file(out, console_id, path);
}

/* Every hash the extension of `path` can give: the console id into `consoles[i]`
 * and its hash into `hashes[i]`. Returns how many (at most `max`). */
int heronite_rc_hash_candidates(const char* path, uint8_t* consoles, char (*hashes)[33], int max)
{
  rc_hash_iterator_t it;
  char hash[33];
  int n = 0;

  rc_hash_initialize_iterator(&it, path, NULL, 0);
  while (n < max && rc_hash_iterate(hash, &it)) {
    consoles[n] = it.consoles[it.index - 1];
    memcpy(hashes[n], hash, 33);
    n++;
  }
  rc_hash_destroy_iterator(&it);
  return n;
}
