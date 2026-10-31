// Internal helpers: parallel_for, function multiversioning, scratch buffers.
#pragma once
#include <algorithm>
#include <atomic>
#include <cstddef>
#include <thread>
#include <vector>

// Compile hot loops for AVX2+FMA and a baseline, chosen at load time (x86-64 Linux, GCC/clang ELF).
#if defined(__x86_64__) && defined(__linux__) && (defined(__GNUC__) || defined(__clang__)) && !defined(HV_NO_MULTIVERSION)
#  define HV_MV __attribute__((target_clones("arch=haswell", "default")))
#else
#  define HV_MV
#endif

namespace hv {

inline int resolve_threads(int t) {
    if (t <= 0) t = (int)std::thread::hardware_concurrency();
    return std::max(1, t);
}

// Runs fn(begin, end) over [0, n) in `chunk`-sized pieces on up to `threads` threads.
// Threads are created per call (tens of microseconds): no global pool, so concurrent callers never interact.
template <class F>
void parallel_for(int n, int threads, int chunk, F &&fn) {
    threads = std::min(resolve_threads(threads), std::max(1, (n + chunk - 1) / chunk));
    if (threads == 1) { fn(0, n); return; }
    std::atomic<int> next{0};
    auto worker = [&] {
        for (;;) {
            int b = next.fetch_add(chunk);
            if (b >= n) return;
            fn(b, std::min(n, b + chunk));
        }
    };
    std::vector<std::thread> pool;
    pool.reserve(threads - 1);
    for (int i = 1; i < threads; ++i) pool.emplace_back(worker);
    worker();
    for (auto &t : pool) t.join();
}

// Per-thread reusable float buffer: avoids ~70 MB of page-faulting allocation per 1080p frame.
inline float *scratch(int slot, size_t n) {
    thread_local std::vector<float> bufs[6];
    auto &b = bufs[slot];
    if (b.size() < n) b.resize(n);
    return b.data();
}

}  // namespace hv
