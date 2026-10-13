/* Copyright 2014 Google Inc. All Rights Reserved.

   Distributed under MIT license.
   See file LICENSE for detail or copy at https://opensource.org/licenses/MIT
*/

/* The helper functions exported by libwoff2common. They were never declared
   in an installed header, but they are exported by the library (and used,
   e.g., by woff2_info), so they are kept with exactly the same signatures.
   This replaces the original private headers table_tags.h,
   variable_length.h and woff2_common.h. */

#ifndef WOFF2_COMMON_ABI_H_
#define WOFF2_COMMON_ABI_H_

#include <stddef.h>
#include <inttypes.h>
#include <vector>

#include "./buffer.h"

namespace woff2 {

// table_tags.h
extern const uint32_t kKnownTags[];

// variable_length.h
size_t Size255UShort(uint16_t value);
bool Read255UShort(Buffer* buf, unsigned int* value);
void Write255UShort(std::vector<uint8_t>* out, int value);
void Store255UShort(int val, size_t* offset, uint8_t* dst);

size_t Base128Size(size_t n);
bool ReadBase128(Buffer* buf, uint32_t* value);
void StoreBase128(size_t len, size_t* offset, uint8_t* dst);

// woff2_common.h
size_t CollectionHeaderSize(uint32_t header_version, uint32_t num_fonts);
uint32_t ComputeULongSum(const uint8_t* buf, size_t size);

} // namespace woff2

#endif  // WOFF2_COMMON_ABI_H_
