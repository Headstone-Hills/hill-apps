// Internal helpers: parallel_for, function multiversioning, aligned buffers, scratch space.
//
// Deliberately free of the C++ runtime (no std::vector/std::thread/exceptions/guarded statics): the library is
// cross-compiled for macOS with a *static* libc++, and any runtime symbol it pulled in would be exported from
// the dylib (operator new/delete...) where dyld could coalesce it with the host process's own copy.
#pragma once
#include <algorithm>
#include <atomic>
#include <cstddef>
#include <cstdlib>
#include <cstring>
#include <type_traits>

#if defined(_WIN32)
#  include <thread>
#  include <vector>
#else
#  include <pthread.h>
#  include <unistd.h>
#endif

// Compile hot loops for AVX2+FMA and a baseline, chosen at load time (x86-64 Linux, GCC/clang ELF).
#if defined(__x86_64__) && defined(__linux__) && (defined(__GNUC__) || defined(__clang__)) && !defined(HV_NO_MULTIVERSION)
#  define HV_MV __attribute__((target_clones("arch=haswell", "default")))
#else
#  define HV_MV
#endif

namespace hv {

inline int hw_threads() {
#if defined(_WIN32)
    return (int)std::thread::hardware_concurrency();
#else
    long n = sysconf(_SC_NPROCESSORS_ONLN);
    return n > 0 ? (int)n : 1;
#endif
}

inline int resolve_threads(int t) {
    if (t <= 0) t = hw_threads();
    return std::max(1, std::min(t, 64));
}

// Owning float buffer, 64-byte aligned, malloc-backed. ok() is false if allocation failed.
struct FBuf {
    float *p = nullptr;
    size_t cap = 0;
    FBuf() = default;
    explicit FBuf(size_t n) { reserve(n); }
    FBuf(const FBuf &) = delete;
    FBuf &operator=(const FBuf &) = delete;
    ~FBuf() { std::free(p); }
    bool reserve(size_t n) {
        if (n <= cap) return true;
        std::free(p);
        p = nullptr;
        cap = 0;
        size_t bytes = (n * sizeof(float) + 63) & ~(size_t)63;
        void *q = nullptr;
#if defined(_WIN32)
        q = _aligned_malloc(bytes, 64);
#else
        if (posix_memalign(&q, 64, bytes) != 0) q = nullptr;
#endif
        if (!q) return false;
        p = (float *)q;
        cap = n;
        return true;
    }
    bool ok() const { return p != nullptr; }
    float *get() const { return p; }
};

#if !defined(_WIN32)
template <class F>
struct PFState {
    F *fn;
    std::atomic<int> *next;
    int n, chunk;
};
template <class F>
void *pf_trampoline(void *arg) {
    PFState<F> *s = (PFState<F> *)arg;
    for (;;) {
        int b = s->next->fetch_add(s->chunk);
        if (b >= s->n) break;
        (*s->fn)(b, std::min(s->n, b + s->chunk));
    }
    return nullptr;
}
#endif

// Runs fn(begin, end) over [0, n) in `chunk`-sized pieces on up to `threads` threads (this one included).
// Threads are created per call (tens of microseconds): no global pool, so concurrent callers never interact.
// If a thread cannot be created the remaining ones simply do more of the work.
template <class F>
void parallel_for(int n, int threads, int chunk, F &&fn) {
    threads = std::min(resolve_threads(threads), std::max(1, (n + chunk - 1) / chunk));
    if (threads == 1) { fn(0, n); return; }
    std::atomic<int> next{0};
#if defined(_WIN32)
    auto worker = [&] {
        for (;;) {
            int b = next.fetch_add(chunk);
            if (b >= n) return;
            fn(b, std::min(n, b + chunk));
        }
    };
    std::vector<std::thread> pool;
    for (int i = 1; i < threads; ++i) pool.emplace_back(worker);
    worker();
    for (auto &t : pool) t.join();
#else
    using S = PFState<typename std::remove_reference<F>::type>;
    S state{&fn, &next, n, chunk};
    pthread_t tid[64];
    bool started[64] = {false};
    for (int i = 1; i < threads; ++i)
        started[i] = pthread_create(&tid[i], nullptr, pf_trampoline<typename std::remove_reference<F>::type>, &state) == 0;
    pf_trampoline<typename std::remove_reference<F>::type>(&state);
    for (int i = 1; i < threads; ++i)
        if (started[i]) pthread_join(tid[i], nullptr);
#endif
}

// Per-thread reusable float buffers: avoids ~70 MB of page-faulting allocation per 1080p frame.
// Returns nullptr if memory is exhausted.
inline float *scratch(int slot, size_t n) {
    thread_local FBuf bufs[6];
    FBuf &b = bufs[slot];
    return b.reserve(n) ? b.get() : nullptr;
}

}  // namespace hv
