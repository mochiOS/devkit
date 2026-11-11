#include <mochi/bundle.h>
#include <type_traits>
static_assert(std::is_same_v<decltype(&mochi_bundle_check), int (*)(const char*)>);
static_assert(std::is_same_v<decltype(&mochi_bundle_open), int (*)(const char*, const char*)>);
static_assert(std::is_same_v<decltype(&mochi_bundle_directory_open), uint64_t (*)(const char*, char*, size_t)>);
static_assert(std::is_same_v<decltype(&mochi_bundle_directory_close), int (*)(uint64_t)>);
extern "C" int sdk_bundle_probe(void);
int main() { return sdk_bundle_probe(); }
