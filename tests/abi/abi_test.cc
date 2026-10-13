// ABI compatibility test.
//
// Build this against the *original* C++ woff2 headers and libraries, then run
// the resulting binary, unchanged, against the new (Rust-based) libraries. It
// exercises every public entry point, a user-defined WOFF2Out subclass
// (vtable/ABI of the abstract interface), exception propagation through the
// decoder, WOFF2StringOut/WOFF2MemoryOut, and the libwoff2common helpers.
//
// It prints a digest of all observable results, so the output of the binary
// running against the old and against the new libraries can be compared.
//
// usage: abi_test <font.ttf/otf/ttc>...

#include <stdint.h>
#include <stdio.h>

#include <fstream>
#include <iterator>
#include <stdexcept>
#include <string>
#include <vector>

#include <woff2/decode.h>
#include <woff2/encode.h>
#include <woff2/output.h>

// Exported by libwoff2common / libwoff2enc, but not declared in the
// installed headers.
namespace woff2 {
class Buffer;
uint32_t ComputeULongSum(const uint8_t* buf, size_t size);
size_t CollectionHeaderSize(uint32_t header_version, uint32_t num_fonts);
size_t Size255UShort(uint16_t value);
void Write255UShort(std::vector<uint8_t>* out, int value);
void Store255UShort(int val, size_t* offset, uint8_t* dst);
size_t Base128Size(size_t n);
void StoreBase128(size_t len, size_t* offset, uint8_t* dst);
extern const uint32_t kKnownTags[];
uint32_t CompressedBufferSize(uint32_t original_size);
}  // namespace woff2

namespace {

uint64_t Hash(const void* p, size_t n, uint64_t h = 1469598103934665603ULL) {
  const uint8_t* b = static_cast<const uint8_t*>(p);
  for (size_t i = 0; i < n; ++i) {
    h ^= b[i];
    h *= 1099511628211ULL;
  }
  return h;
}

std::string Read(const char* path) {
  std::ifstream ifs(path, std::ios::binary);
  return std::string(std::istreambuf_iterator<char>(ifs.rdbuf()),
                     std::istreambuf_iterator<char>());
}

// A user-defined output that records calls and can throw.
class RecordingOut : public woff2::WOFF2Out {
 public:
  explicit RecordingOut(long throw_at) : throw_at_(throw_at) {}
  bool Write(const void* buf, size_t n) override {
    return Write(buf, data.size() > offset_ ? offset_ : data.size(), n);
  }
  bool Write(const void* buf, size_t offset, size_t n) override {
    Tick();
    digest = Hash(&offset, sizeof(offset), Hash(buf, n, digest));
    if (offset + n > data.size()) data.resize(offset + n);
    data.replace(offset, n, static_cast<const char*>(buf), n);
    if (offset + n > offset_) offset_ = offset + n;
    return true;
  }
  size_t Size() override {
    Tick();
    return offset_;
  }
  std::string data;
  uint64_t digest = 0;
  long calls = 0;

 private:
  void Tick() {
    if (++calls == throw_at_) throw std::runtime_error("boom");
  }
  long throw_at_;
  size_t offset_ = 0;
};

}  // namespace

int main(int argc, char** argv) {
  // libwoff2common helpers.
  uint64_t h = 0;
  for (int i = 0; i < 63; ++i) h = Hash(&woff2::kKnownTags[i], 4, h);
  printf("kKnownTags %016llx\n", (unsigned long long)h);
  std::vector<uint8_t> v;
  uint8_t dst[64];
  size_t off = 0;
  for (int x : {0, 252, 253, 505, 506, 761, 762, 65535}) {
    woff2::Write255UShort(&v, x);
    woff2::Store255UShort(x, &off, dst);
    printf("255UShort(%d) size=%zu\n", x, woff2::Size255UShort(x));
  }
  off = 0;
  woff2::StoreBase128(123456789, &off, dst);
  printf("Base128 %zu %zu %016llx %016llx\n", woff2::Base128Size(123456789),
         off, (unsigned long long)Hash(v.data(), v.size()),
         (unsigned long long)Hash(dst, off));
  printf("ULongSum %08x CollectionHeaderSize %zu CompressedBufferSize %u\n",
         woff2::ComputeULongSum(v.data(), v.size()),
         woff2::CollectionHeaderSize(0x00020000, 7),
         woff2::CompressedBufferSize(1000));

  for (int a = 1; a < argc; ++a) {
    std::string font = Read(argv[a]);
    const uint8_t* data = reinterpret_cast<const uint8_t*>(font.data());

    // Encoding, both overloads.
    for (int variant = 0; variant < 3; ++variant) {
      woff2::WOFF2Params params;
      if (variant == 1) {
        params.brotli_quality = 5;
        params.allow_transforms = false;
        params.extended_metadata = "<?xml version=\"1.0\"?><metadata/>";
      }
      size_t max = variant == 1
                       ? woff2::MaxWOFF2CompressedSize(
                             data, font.size(), params.extended_metadata)
                       : woff2::MaxWOFF2CompressedSize(data, font.size());
      std::string out(max, 0);
      size_t len = variant == 2 ? 100 : max;
      bool ok =
          variant == 0
              ? woff2::ConvertTTFToWOFF2(data, font.size(),
                                         reinterpret_cast<uint8_t*>(&out[0]),
                                         &len)
              : woff2::ConvertTTFToWOFF2(data, font.size(),
                                         reinterpret_cast<uint8_t*>(&out[0]),
                                         &len, params);
      out.resize(len);
      printf("%s encode[%d] max=%zu ok=%d len=%zu hash=%016llx\n", argv[a],
             variant, max, ok, len,
             (unsigned long long)Hash(out.data(), out.size()));
      if (!ok || variant == 2) continue;

      const uint8_t* w = reinterpret_cast<const uint8_t*>(out.data());
      size_t final_size = woff2::ComputeWOFF2FinalSize(w, out.size());

      // Decoding into a WOFF2StringOut.
      std::string ttf(std::min(final_size, woff2::kDefaultMaxSize), 0);
      woff2::WOFF2StringOut sout(&ttf);
      ok = woff2::ConvertWOFF2ToTTF(w, out.size(), &sout);
      printf("  decode string ok=%d size=%zu max=%zu hash=%016llx\n", ok,
             sout.Size(), sout.MaxSize(),
             (unsigned long long)Hash(ttf.data(), sout.Size()));
      std::string small;
      woff2::WOFF2StringOut sout2(&small);
      sout2.SetMaxSize(sout.Size() / 2);
      printf("  decode limited ok=%d\n",
             woff2::ConvertWOFF2ToTTF(w, out.size(), &sout2));

      // The deprecated fixed buffer API (WOFF2MemoryOut).
      std::vector<uint8_t> mem(final_size);
      ok = woff2::ConvertWOFF2ToTTF(mem.data(), mem.size(), w, out.size());
      printf("  decode memory ok=%d final=%zu hash=%016llx\n", ok, final_size,
             (unsigned long long)Hash(mem.data(), mem.size()));

      // A user-defined WOFF2Out.
      RecordingOut rec(-1);
      ok = woff2::ConvertWOFF2ToTTF(w, out.size(), &rec);
      printf("  decode custom ok=%d calls=%ld digest=%016llx\n", ok, rec.calls,
             (unsigned long long)rec.digest);

      // Exceptions thrown by the WOFF2Out propagate to the caller.
      for (long t : {1L, rec.calls / 2, rec.calls}) {
        RecordingOut thrower(t);
        try {
          ok = woff2::ConvertWOFF2ToTTF(w, out.size(), &thrower);
          printf("  throw_at=%ld: no exception, ok=%d\n", t, ok);
        } catch (const std::runtime_error& e) {
          printf("  throw_at=%ld: caught '%s' after %ld calls digest=%016llx\n",
                 t, e.what(), thrower.calls,
                 (unsigned long long)thrower.digest);
        }
      }
    }
  }
  return 0;
}
