/* Distributed under MIT license.
   See file LICENSE for detail or copy at https://opensource.org/licenses/MIT
*/

/* libwoff2enc: C++ ABI of the WOFF2 encoder, implemented in Rust. */

#include <woff2/encode.h>

#include <brotli/encode.h>
#include "./woff2_rs.h"

namespace woff2 {

// Not declared in any public header, but exported by the original library.
uint32_t CompressedBufferSize(uint32_t original_size);

namespace {

int Compress(int quality, int lgwin, int mode, size_t input_size,
             const uint8_t* input_buffer, size_t* encoded_size,
             uint8_t* encoded_buffer) {
  return BrotliEncoderCompress(quality, lgwin,
                               static_cast<BrotliEncoderMode>(mode),
                               input_size, input_buffer, encoded_size,
                               encoded_buffer);
}

}  // namespace

size_t MaxWOFF2CompressedSize(const uint8_t* data, size_t length) {
  return MaxWOFF2CompressedSize(data, length, "");
}

size_t MaxWOFF2CompressedSize(const uint8_t* data, size_t length,
                              const std::string& extended_metadata) {
  return woff2_rs_max_compressed_size(data, length,
                                      extended_metadata.length());
}

uint32_t CompressedBufferSize(uint32_t original_size) {
  return woff2_rs_compressed_buffer_size(original_size);
}

bool ConvertTTFToWOFF2(const uint8_t *data, size_t length,
                       uint8_t *result, size_t *result_length) {
  WOFF2Params params;
  return ConvertTTFToWOFF2(data, length, result, result_length,
                           params);
}

bool ConvertTTFToWOFF2(const uint8_t *data, size_t length,
                       uint8_t *result, size_t *result_length,
                       const WOFF2Params& params) {
  return woff2_rs_convert_ttf_to_woff2(
      data, length, result, result_length,
      reinterpret_cast<const uint8_t*>(params.extended_metadata.data()),
      params.extended_metadata.length(), params.brotli_quality,
      params.allow_transforms, &Compress);
}

} // namespace woff2
