#if !defined(__mochios__) || defined(__linux__)
#error A host Linux compilation cannot satisfy the mochiOS probe
#endif
#include <atomic>
#include <stdexcept>
#include <thread>
#include <vector>
static_assert(__cplusplus > 202002L);
extern "C" int mochi_sdk_probe() {
    std::atomic<int> value{0};
    std::thread worker([&] { value.store(42); });
    worker.join();
    std::vector<int> values{value.load()};
    try { throw std::runtime_error("shared unwind probe"); }
    catch (const std::exception&) { return values.front(); }
}
