/* Distributed under MIT license.
   See file LICENSE for detail or copy at https://opensource.org/licenses/MIT
*/

/* Private C interface between the C++ ABI layer and the Rust implementation
   (src/capi.rs). The Rust code lives in libwoff2common; libwoff2dec and
   libwoff2enc call into it and pass in the brotli entry points they link
   against, so that the dependency structure of the libraries is unchanged.

   This is NOT a public interface. */

#ifndef WOFF2_RS_H_
#define WOFF2_RS_H_

#include <stddef.h>
#include <stdint.h>
#ifndef __cplusplus
#include <stdbool.h>
#endif

#ifdef __cplusplus
extern "C" {
#endif

/* Callbacks forwarding to a woff2::WOFF2Out. */
typedef struct woff2_rs_out {
  void* ctx;
  bool (*write)(void* ctx, const uint8_t* buf, size_t n);
  bool (*write_at)(void* ctx, const uint8_t* buf, size_t offset, size_t n);
  size_t (*size)(void* ctx);
} woff2_rs_out;

/* Same signature as BrotliDecoderDecompress (BrotliDecoderResult is int). */
typedef int (*woff2_rs_decompress_fn)(size_t encoded_size,
                                      const uint8_t* encoded_buffer,
                                      size_t* decoded_size,
                                      uint8_t* decoded_buffer);

/* Same signature as BrotliEncoderCompress (BrotliEncoderMode is int). */
typedef int (*woff2_rs_compress_fn)(int quality, int lgwin, int mode,
                                    size_t input_size,
                                    const uint8_t* input_buffer,
                                    size_t* encoded_size,
                                    uint8_t* encoded_buffer);

/* libwoff2dec */
size_t woff2_rs_compute_final_size(const uint8_t* data, size_t length);
bool woff2_rs_convert_woff2_to_ttf(const uint8_t* data, size_t length,
                                   const woff2_rs_out* out,
                                   woff2_rs_decompress_fn decompress);

/* libwoff2enc */
size_t woff2_rs_max_compressed_size(const uint8_t* data, size_t length,
                                    size_t extended_metadata_length);
uint32_t woff2_rs_compressed_buffer_size(uint32_t original_size);
bool woff2_rs_convert_ttf_to_woff2(const uint8_t* data, size_t length,
                                   uint8_t* result, size_t* result_length,
                                   const uint8_t* extended_metadata,
                                   size_t extended_metadata_length,
                                   int brotli_quality, bool allow_transforms,
                                   woff2_rs_compress_fn compress);

/* libwoff2common */
uint32_t woff2_rs_compute_ulong_sum(const uint8_t* buf, size_t size);
size_t woff2_rs_collection_header_size(uint32_t header_version,
                                       uint32_t num_fonts);
size_t woff2_rs_size_255_ushort(uint16_t value);
size_t woff2_rs_encode_255_ushort(int value, uint8_t out[3]);
size_t woff2_rs_base128_size(size_t n);
size_t woff2_rs_encode_base128(size_t len, uint8_t out[10]);
bool woff2_rs_read_255_ushort(const uint8_t* data, size_t length,
                              size_t* offset, unsigned int* value);
bool woff2_rs_read_base128(const uint8_t* data, size_t length,
                           size_t* offset, uint32_t* value);

#ifdef __cplusplus
}  /* extern "C" */
#endif

#endif  /* WOFF2_RS_H_ */
