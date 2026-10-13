/* Distributed under MIT license.
   See file LICENSE for detail or copy at https://opensource.org/licenses/MIT
*/

/* libwoff2dec: C++ ABI of the WOFF2 decoder, implemented in Rust. */

#include <woff2/decode.h>

#include <exception>

#include <brotli/decode.h>
#include "./woff2_rs.h"

namespace woff2 {

namespace {

// Adapts a WOFF2Out (which may be a user-defined subclass) to the C
// callbacks used by the Rust decoder.
//
// Exceptions thrown by the WOFF2Out must not unwind through Rust frames.
// They are caught here, all further calls are short-circuited (so the
// WOFF2Out sees exactly the calls it would have seen before the exception
// propagated in the original implementation), and the exception is rethrown
// once control is back in C++.
struct OutAdapter {
  WOFF2Out* out;
  std::exception_ptr error;
};

bool OutWrite(void* ctx, const uint8_t* buf, size_t n) {
  OutAdapter* adapter = static_cast<OutAdapter*>(ctx);
  if (adapter->error) return false;
  try {
    return adapter->out->Write(buf, n);
  } catch (...) {
    adapter->error = std::current_exception();
    return false;
  }
}

bool OutWriteAt(void* ctx, const uint8_t* buf, size_t offset, size_t n) {
  OutAdapter* adapter = static_cast<OutAdapter*>(ctx);
  if (adapter->error) return false;
  try {
    return adapter->out->Write(buf, offset, n);
  } catch (...) {
    adapter->error = std::current_exception();
    return false;
  }
}

size_t OutSize(void* ctx) {
  OutAdapter* adapter = static_cast<OutAdapter*>(ctx);
  if (adapter->error) return 0;
  try {
    return adapter->out->Size();
  } catch (...) {
    adapter->error = std::current_exception();
    return 0;
  }
}

int Decompress(size_t encoded_size, const uint8_t* encoded_buffer,
               size_t* decoded_size, uint8_t* decoded_buffer) {
  return static_cast<int>(BrotliDecoderDecompress(
      encoded_size, encoded_buffer, decoded_size, decoded_buffer));
}

}  // namespace

size_t ComputeWOFF2FinalSize(const uint8_t* data, size_t length) {
  return woff2_rs_compute_final_size(data, length);
}

bool ConvertWOFF2ToTTF(uint8_t *result, size_t result_length,
                       const uint8_t *data, size_t length) {
  WOFF2MemoryOut out(result, result_length);
  return ConvertWOFF2ToTTF(data, length, &out);
}

bool ConvertWOFF2ToTTF(const uint8_t* data, size_t length,
                       WOFF2Out* out) {
  OutAdapter adapter = {out, nullptr};
  const woff2_rs_out callbacks = {&adapter, &OutWrite, &OutWriteAt, &OutSize};
  const bool ok =
      woff2_rs_convert_woff2_to_ttf(data, length, &callbacks, &Decompress);
  if (adapter.error) {
    std::rethrow_exception(adapter.error);
  }
  return ok;
}

} // namespace woff2
