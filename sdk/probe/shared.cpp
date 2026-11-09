#if !defined(__mochios__) || defined(__linux__)
#error A host Linux compilation cannot satisfy the mochiOS probe
#endif
#include <atomic>
#include <stdexcept>
#include <thread>
#include <vector>
static_assert(__cplusplus > 202002L);
// Volatile operands keep Clang's 128-bit division/remainder helpers in the
// shared link; a syntax-only check cannot detect missing compiler builtins.
extern "C" __int128 mochi_sdk_signed_divide(__int128 a, __int128 b) { return a / b; }
extern "C" __int128 mochi_sdk_signed_remainder(__int128 a, __int128 b) { return a % b; }
extern "C" unsigned __int128 mochi_sdk_unsigned_divide(unsigned __int128 a, unsigned __int128 b) { return a / b; }
extern "C" unsigned __int128 mochi_sdk_unsigned_remainder(unsigned __int128 a, unsigned __int128 b) { return a % b; }
extern "C" int mochi_sdk_probe() {
    std::atomic<int> value{0};
    std::thread worker([&] { value.store(42); });
    worker.join();
    std::vector<int> values{value.load()};
    try { throw std::runtime_error("shared unwind probe"); }
    catch (const std::exception&) { return values.front(); }
}
