// Glue exposing the woff2 C++ API through plain C functions.
//
// This file is compiled twice:
//  * against the original C++ implementation (fuzz/reference), with
//    -Dwoff2=woff2_ref -DGLUE_REF -DGLUE_PREFIX=ref_
//  * against the new implementation (cpp/ ABI layer + Rust), with
//    -DGLUE_PREFIX=new_
// so that both can be linked into one fuzzer binary and compared through
// exactly the public C++ API that existing users call.

#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include <stdexcept>
#include <string>
#include <vector>

#include <woff2/decode.h>
#include <woff2/encode.h>
#include <woff2/output.h>

#ifdef GLUE_REF
#include "./buffer.h"
#include "./table_tags.h"
#include "./variable_length.h"
#include "./woff2_common.h"
#else
#include "./woff2_common_abi.h"
#endif

namespace woff2 {
// Exported by libwoff2enc, but not declared in any header.
uint32_t CompressedBufferSize(uint32_t original_size);
}  // namespace woff2

#define GLUE_CAT2(a, b) a##b
#define GLUE_CAT(a, b) GLUE_CAT2(a, b)
#define GLUE(name) GLUE_CAT(GLUE_PREFIX, name)

extern "C" {

struct glue_buf {
  uint8_t* data;
  size_t len;
};

}  // extern "C"

namespace {

void SetBuf(glue_buf* b, const void* p, size_t n) {
  b->data = static_cast<uint8_t*>(malloc(n ? n : 1));
  if (n) memcpy(b->data, p, n);
  b->len = n;
}

uint64_t Hash(const void* p, size_t n) {
  const uint8_t* b = static_cast<const uint8_t*>(p);
  uint64_t h = 1469598103934665603ULL;
  for (size_t i = 0; i < n; ++i) {
    h ^= b[i];
    h *= 1099511628211ULL;
  }
  return h;
}

void Put64(std::string* s, uint64_t v) {
  s->append(reinterpret_cast<const char*>(&v), sizeof(v));
}

// A user-defined WOFF2Out that records every call made on it. It can fail
// (return false) or throw at a given call number, and caps its size.
class TraceOut : public woff2::WOFF2Out {
 public:
  TraceOut(int64_t fail_at, int64_t throw_at, size_t max_size)
      : fail_at_(fail_at), throw_at_(throw_at), max_size_(max_size) {}

  bool Write(const void* buf, size_t n) override {
    Event('A', offset_, n, Hash(buf, n));
    return DoWrite(buf, offset_, n);
  }

  bool Write(const void* buf, size_t offset, size_t n) override {
    Event('W', offset, n, Hash(buf, n));
    return DoWrite(buf, offset, n);
  }

  size_t Size() override {
    Event('S', offset_, 0, 0);
    Check();
    return offset_;
  }

  std::string trace;
  std::string contents;

 private:
  void Event(char type, size_t offset, size_t n, uint64_t hash) {
    trace.push_back(type);
    Put64(&trace, offset);
    Put64(&trace, n);
    Put64(&trace, hash);
  }

  void Check() {
    ++calls_;
    if (calls_ == throw_at_) throw std::runtime_error("TraceOut");
  }

  bool DoWrite(const void* buf, size_t offset, size_t n) {
    Check();
    if (calls_ == fail_at_) return false;
    if (offset > max_size_ || n > max_size_ - offset) return false;
    if (offset + n > contents.size()) contents.resize(offset + n);
    if (n) memcpy(&contents[offset], buf, n);
    if (offset + n > offset_) offset_ = offset + n;
    return true;
  }

  int64_t calls_ = 0;
  int64_t fail_at_;
  int64_t throw_at_;
  size_t max_size_;
  size_t offset_ = 0;
};

}  // namespace

extern "C" {

size_t GLUE(final_size)(const uint8_t* data, size_t len) {
  return woff2::ComputeWOFF2FinalSize(data, len);
}

int GLUE(decode_string)(const uint8_t* data, size_t len, size_t initial_size,
                        int set_max, size_t max_size, glue_buf* contents,
                        size_t* out_size) {
  std::string output(initial_size, 0);
  woff2::WOFF2StringOut out(&output);
  if (set_max) out.SetMaxSize(max_size);
  bool ok = woff2::ConvertWOFF2ToTTF(data, len, &out);
  SetBuf(contents, output.data(), output.size());
  *out_size = out.Size();
  return ok;
}

int GLUE(decode_memory)(uint8_t* result, size_t result_length,
                        const uint8_t* data, size_t len) {
  return woff2::ConvertWOFF2ToTTF(result, result_length, data, len);
}

int GLUE(decode_trace)(const uint8_t* data, size_t len, int64_t fail_at,
                       int64_t throw_at, size_t max_size, glue_buf* trace,
                       glue_buf* contents, int* threw) {
  TraceOut out(fail_at, throw_at, max_size);
  bool ok = false;
  *threw = 0;
  try {
    ok = woff2::ConvertWOFF2ToTTF(data, len, &out);
  } catch (const std::runtime_error&) {
    *threw = 1;
  }
  SetBuf(trace, out.trace.data(), out.trace.size());
  SetBuf(contents, out.contents.data(), out.contents.size());
  return ok;
}

size_t GLUE(max_compressed_size)(const uint8_t* data, size_t len,
                                 const uint8_t* meta, size_t meta_len,
                                 int overload) {
  if (overload == 0) return woff2::MaxWOFF2CompressedSize(data, len);
  return woff2::MaxWOFF2CompressedSize(
      data, len, std::string(reinterpret_cast<const char*>(meta), meta_len));
}

uint32_t GLUE(compressed_buffer_size)(uint32_t original_size) {
  return woff2::CompressedBufferSize(original_size);
}

int GLUE(encode)(const uint8_t* data, size_t len, uint8_t* result,
                 size_t* result_length, const uint8_t* meta, size_t meta_len,
                 int quality, int allow_transforms, int overload) {
  if (overload == 0) {
    return woff2::ConvertTTFToWOFF2(data, len, result, result_length);
  }
  woff2::WOFF2Params params;
  params.extended_metadata.assign(reinterpret_cast<const char*>(meta),
                                  meta_len);
  params.brotli_quality = quality;
  params.allow_transforms = allow_transforms != 0;
  return woff2::ConvertTTFToWOFF2(data, len, result, result_length, params);
}

// libwoff2common helpers.

int GLUE(read_255_ushort)(const uint8_t* data, size_t len, size_t start,
                          size_t* offset, unsigned int* value) {
  woff2::Buffer buf(data, len);
  buf.set_offset(start);
  *value = 0xdeadbeef;
  bool ok = woff2::Read255UShort(&buf, value);
  *offset = buf.offset();
  return ok;
}

int GLUE(read_base128)(const uint8_t* data, size_t len, size_t start,
                       size_t* offset, uint32_t* value) {
  woff2::Buffer buf(data, len);
  buf.set_offset(start);
  *value = 0xdeadbeef;
  bool ok = woff2::ReadBase128(&buf, value);
  *offset = buf.offset();
  return ok;
}

// out must have room for 8 bytes.
size_t GLUE(write_255_ushort)(int value, uint8_t* out) {
  std::vector<uint8_t> v;
  v.push_back(0x77);
  woff2::Write255UShort(&v, value);
  memcpy(out, v.data(), v.size());
  return v.size();
}

// out must have room for start + 3 bytes.
size_t GLUE(store_255_ushort)(int value, size_t start, uint8_t* out) {
  size_t offset = start;
  woff2::Store255UShort(value, &offset, out);
  return offset;
}

size_t GLUE(size_255_ushort)(uint16_t value) {
  return woff2::Size255UShort(value);
}

size_t GLUE(base128_size)(size_t n) { return woff2::Base128Size(n); }

// out must have room for start + 10 bytes.
size_t GLUE(store_base128)(size_t len, size_t start, uint8_t* out) {
  size_t offset = start;
  woff2::StoreBase128(len, &offset, out);
  return offset;
}

uint32_t GLUE(ulong_sum)(const uint8_t* buf, size_t size) {
  return woff2::ComputeULongSum(buf, size);
}

size_t GLUE(collection_header_size)(uint32_t header_version,
                                    uint32_t num_fonts) {
  return woff2::CollectionHeaderSize(header_version, num_fonts);
}

const uint32_t* GLUE(known_tags)(void) { return woff2::kKnownTags; }

}  // extern "C"
