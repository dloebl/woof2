/* Copyright 2014 Google Inc. All Rights Reserved.

   Distributed under MIT license.
   See file LICENSE for detail or copy at https://opensource.org/licenses/MIT
*/

/* libwoff2common: C++ ABI for the shared helpers, implemented in Rust. */

#include "./woff2_common_abi.h"
#include "./woff2_rs.h"

namespace woff2 {

// Note that the byte order is big-endian, not the same as ots.cc
#define TAG(a, b, c, d) ((a << 24) | (b << 16) | (c << 8) | d)

// Must match KNOWN_TAGS in src/table_tags.rs (checked by the fuzz harness).
const uint32_t kKnownTags[63] = {
  TAG('c', 'm', 'a', 'p'),  // 0
  TAG('h', 'e', 'a', 'd'),  // 1
  TAG('h', 'h', 'e', 'a'),  // 2
  TAG('h', 'm', 't', 'x'),  // 3
  TAG('m', 'a', 'x', 'p'),  // 4
  TAG('n', 'a', 'm', 'e'),  // 5
  TAG('O', 'S', '/', '2'),  // 6
  TAG('p', 'o', 's', 't'),  // 7
  TAG('c', 'v', 't', ' '),  // 8
  TAG('f', 'p', 'g', 'm'),  // 9
  TAG('g', 'l', 'y', 'f'),  // 10
  TAG('l', 'o', 'c', 'a'),  // 11
  TAG('p', 'r', 'e', 'p'),  // 12
  TAG('C', 'F', 'F', ' '),  // 13
  TAG('V', 'O', 'R', 'G'),  // 14
  TAG('E', 'B', 'D', 'T'),  // 15
  TAG('E', 'B', 'L', 'C'),  // 16
  TAG('g', 'a', 's', 'p'),  // 17
  TAG('h', 'd', 'm', 'x'),  // 18
  TAG('k', 'e', 'r', 'n'),  // 19
  TAG('L', 'T', 'S', 'H'),  // 20
  TAG('P', 'C', 'L', 'T'),  // 21
  TAG('V', 'D', 'M', 'X'),  // 22
  TAG('v', 'h', 'e', 'a'),  // 23
  TAG('v', 'm', 't', 'x'),  // 24
  TAG('B', 'A', 'S', 'E'),  // 25
  TAG('G', 'D', 'E', 'F'),  // 26
  TAG('G', 'P', 'O', 'S'),  // 27
  TAG('G', 'S', 'U', 'B'),  // 28
  TAG('E', 'B', 'S', 'C'),  // 29
  TAG('J', 'S', 'T', 'F'),  // 30
  TAG('M', 'A', 'T', 'H'),  // 31
  TAG('C', 'B', 'D', 'T'),  // 32
  TAG('C', 'B', 'L', 'C'),  // 33
  TAG('C', 'O', 'L', 'R'),  // 34
  TAG('C', 'P', 'A', 'L'),  // 35
  TAG('S', 'V', 'G', ' '),  // 36
  TAG('s', 'b', 'i', 'x'),  // 37
  TAG('a', 'c', 'n', 't'),  // 38
  TAG('a', 'v', 'a', 'r'),  // 39
  TAG('b', 'd', 'a', 't'),  // 40
  TAG('b', 'l', 'o', 'c'),  // 41
  TAG('b', 's', 'l', 'n'),  // 42
  TAG('c', 'v', 'a', 'r'),  // 43
  TAG('f', 'd', 's', 'c'),  // 44
  TAG('f', 'e', 'a', 't'),  // 45
  TAG('f', 'm', 't', 'x'),  // 46
  TAG('f', 'v', 'a', 'r'),  // 47
  TAG('g', 'v', 'a', 'r'),  // 48
  TAG('h', 's', 't', 'y'),  // 49
  TAG('j', 'u', 's', 't'),  // 50
  TAG('l', 'c', 'a', 'r'),  // 51
  TAG('m', 'o', 'r', 't'),  // 52
  TAG('m', 'o', 'r', 'x'),  // 53
  TAG('o', 'p', 'b', 'd'),  // 54
  TAG('p', 'r', 'o', 'p'),  // 55
  TAG('t', 'r', 'a', 'k'),  // 56
  TAG('Z', 'a', 'p', 'f'),  // 57
  TAG('S', 'i', 'l', 'f'),  // 58
  TAG('G', 'l', 'a', 't'),  // 59
  TAG('G', 'l', 'o', 'c'),  // 60
  TAG('F', 'e', 'a', 't'),  // 61
  TAG('S', 'i', 'l', 'l'),  // 62
};

#undef TAG

uint32_t ComputeULongSum(const uint8_t* buf, size_t size) {
  return woff2_rs_compute_ulong_sum(buf, size);
}

size_t CollectionHeaderSize(uint32_t header_version, uint32_t num_fonts) {
  return woff2_rs_collection_header_size(header_version, num_fonts);
}

size_t Size255UShort(uint16_t value) {
  return woff2_rs_size_255_ushort(value);
}

void Write255UShort(std::vector<uint8_t>* out, int value) {
  uint8_t packed[3];
  size_t n = woff2_rs_encode_255_ushort(value, packed);
  for (size_t i = 0; i < n; ++i) {
    out->push_back(packed[i]);
  }
}

void Store255UShort(int val, size_t* offset, uint8_t* dst) {
  uint8_t packed[3];
  size_t n = woff2_rs_encode_255_ushort(val, packed);
  for (size_t i = 0; i < n; ++i) {
    dst[(*offset)++] = packed[i];
  }
}

bool Read255UShort(Buffer* buf, unsigned int* value) {
  size_t offset = buf->offset();
  bool ok = woff2_rs_read_255_ushort(buf->buffer(), buf->length(), &offset,
                                     value);
  buf->set_offset(offset);
  return ok;
}

size_t Base128Size(size_t n) {
  return woff2_rs_base128_size(n);
}

bool ReadBase128(Buffer* buf, uint32_t* value) {
  size_t offset = buf->offset();
  bool ok = woff2_rs_read_base128(buf->buffer(), buf->length(), &offset,
                                  value);
  buf->set_offset(offset);
  return ok;
}

void StoreBase128(size_t len, size_t* offset, uint8_t* dst) {
  uint8_t packed[10];
  size_t n = woff2_rs_encode_base128(len, packed);
  for (size_t i = 0; i < n; ++i) {
    dst[(*offset)++] = packed[i];
  }
}

} // namespace woff2
