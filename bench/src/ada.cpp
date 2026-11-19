// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

// Ada's C API parses the base again on every call with one, which the
// other two parsers don't, so the benchmark parses against a base Ada
// already parsed, through its C++ API.

#include "ada.h"

#include <string_view>

// Whether `input` parses, against `base` (an `ada_url` from `ada_parse`)
// unless it's null. The URL is built and freed, as for the others.
extern "C" bool parsec_bench_ada_parse(const char* input, size_t length,
                                       const void* base) {
  std::string_view in(input, length);
  if (base == nullptr) {
    return ada::parse<ada::url_aggregator>(in).has_value();
  }
  auto& b = *static_cast<const ada::result<ada::url_aggregator>*>(base);
  if (!b) {
    return false;
  }
  return ada::parse<ada::url_aggregator>(in, &*b).has_value();
}
