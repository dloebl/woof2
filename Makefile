OS := $(shell uname)

CPPFLAGS = -I./brotli/c/include/ -I./cpp -I./include

AR ?= ar
CC ?= gcc
CXX ?= g++
CARGO ?= cargo

# It's helpful to be able to turn these off for fuzzing
CANONICAL_PREFIXES ?= -no-canonical-prefixes
NOISY_LOGGING ?= -DFONT_COMPRESSION_BIN
COMMON_FLAGS = -fno-omit-frame-pointer $(CANONICAL_PREFIXES) $(NOISY_LOGGING) -D __STDC_FORMAT_MACROS

ARFLAGS = crf

ifeq ($(OS), Darwin)
  CPPFLAGS += -DOS_MACOSX
  ARFLAGS = cr
else
  COMMON_FLAGS += -fno-tree-vrp
endif


CFLAGS += $(COMMON_FLAGS)
CXXFLAGS += $(COMMON_FLAGS) -std=c++11

# The C++ ABI layer (the implementation itself is the Rust crate in src/).
SRCDIR = cpp

OUROBJ = common.o decode.o encode.o output.o

# Rust implementation
RUST_FEATURES = capi
ifneq ($(strip $(NOISY_LOGGING)),)
  RUST_FEATURES := $(RUST_FEATURES),noisy-logging
endif
CARGO_TARGET_DIR ?= target
RUSTLIB = $(CARGO_TARGET_DIR)/staticlib/release/libwoff2.a
RUST_SOURCES = Cargo.toml build.rs $(shell find src -name '*.rs')

BROTLI = brotli
BROTLIOBJ = $(BROTLI)/bin/obj/c
ENCOBJ = $(BROTLIOBJ)/enc/*.o
DECOBJ = $(BROTLIOBJ)/dec/*.o
COMMONOBJ = $(BROTLIOBJ)/common/*.o
BROTLILIBDIR = $(CARGO_TARGET_DIR)/brotli

OBJS = $(patsubst %, $(SRCDIR)/%, $(OUROBJ))
EXECUTABLES=woff2_compress woff2_decompress woff2_info
ARCHIVES=convert_woff2ttf_fuzzer convert_woff2ttf_fuzzer_new_entry
ARCHIVE_OBJS=$(patsubst %, $(SRCDIR)/%.o, $(ARCHIVES))
# For compatibility (e.g. with OSS-Fuzz), the archives are written to src/.
ARCHIVEDIR = src

ifeq (,$(wildcard $(BROTLI)/*))
  $(error Brotli dependency not found : you must initialize the Git submodule)
endif

all : $(OBJS) $(EXECUTABLES) $(ARCHIVES)

$(RUSTLIB) : $(RUST_SOURCES)
	CARGO_TARGET_DIR=$(CARGO_TARGET_DIR)/staticlib $(CARGO) rustc --release \
	    --lib --crate-type staticlib --no-default-features \
	    --features $(RUST_FEATURES) -- -C symbol-mangling-version=v0

# Self-contained archives: fuzzer entry point, C++ ABI layer, Rust
# implementation and brotli.
$(ARCHIVES) : $(ARCHIVE_OBJS) $(OBJS) $(RUSTLIB) deps
	rm -f $(ARCHIVEDIR)/$@.a
ifeq ($(OS), Darwin)
	libtool -static -no_warning_for_no_symbols -o $(ARCHIVEDIR)/$@.a $(OBJS) $(COMMONOBJ) $(ENCOBJ) \
	    $(DECOBJ) $(SRCDIR)/$@.o $(RUSTLIB)
else
	( echo "create $(ARCHIVEDIR)/$@.a"; \
	  for o in $(OBJS) $(COMMONOBJ) $(ENCOBJ) $(DECOBJ) $(SRCDIR)/$@.o; do \
	    echo "addmod $$o"; done; \
	  echo "addlib $(RUSTLIB)"; echo "save"; echo "end" ) | $(AR) -M
endif

# The command line tools are written in Rust and link brotli statically.
$(EXECUTABLES) : $(RUST_SOURCES) deps
	mkdir -p $(BROTLILIBDIR)
	rm -f $(BROTLILIBDIR)/*.a
	$(AR) $(ARFLAGS) $(BROTLILIBDIR)/libbrotlienc.a $(ENCOBJ)
	$(AR) $(ARFLAGS) $(BROTLILIBDIR)/libbrotlidec.a $(DECOBJ)
	$(AR) $(ARFLAGS) $(BROTLILIBDIR)/libbrotlicommon.a $(COMMONOBJ)
	CARGO_TARGET_DIR=$(CARGO_TARGET_DIR)/bin \
	    WOFF2_BROTLI_LIB_DIR=$(abspath $(BROTLILIBDIR)) WOFF2_BROTLI_STATIC=1 \
	    $(CARGO) build --release --bin $@ --no-default-features \
	    --features system-brotli$(if $(strip $(NOISY_LOGGING)),$(comma)noisy-logging)
	cp $(CARGO_TARGET_DIR)/bin/release/$@ $@

comma := ,

deps :
	$(MAKE) -C $(BROTLI) lib

clean :
	rm -f $(OBJS) $(ARCHIVE_OBJS) $(EXECUTABLES) \
	    $(patsubst %, $(ARCHIVEDIR)/%.a, $(ARCHIVES))
	rm -rf $(CARGO_TARGET_DIR)/staticlib $(CARGO_TARGET_DIR)/bin $(BROTLILIBDIR)
	$(MAKE) -C $(BROTLI) clean

.PHONY : all deps clean $(ARCHIVES)
