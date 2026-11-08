#define _GNU_SOURCE 1
#include <errno.h>
#include <pthread.h>
#include <stdint.h>
#include <stddef.h>

/* Do not substitute default limits: verify that a local variable lies in the
   observed usable range and that owned guard bytes are outside that range. */
static int inspect(size_t expected_size, size_t expected_guard)
{
    pthread_attr_t observed;
    char local;
    void *base = NULL;
    size_t size = 0, guard = 0;
    int error = pthread_getattr_np(pthread_self(), &observed);
    if (error) return error;
    error = pthread_attr_getstack(&observed, &base, &size);
    if (!error) error = pthread_attr_getguardsize(&observed, &guard);
    if (!error && (!base || !size || (uintptr_t)&local < (uintptr_t)base
        || (uintptr_t)&local - (uintptr_t)base >= size)) error = EINVAL;
    if (!error && expected_size && size != expected_size) error = EINVAL;
    if (!error && guard != expected_guard) error = EINVAL;
    int destroy_error = pthread_attr_destroy(&observed);
    return error ? error : destroy_error;
}

static void *worker(void *argument)
{
    (void)argument;
    return (void *)(uintptr_t)inspect(128 * 1024, 8192);
}

/* Compile/link in CI; a zero result is only a runtime pass when executed inside
   the matching mochiOS guest, never by executing this target binary on Linux. */
int sdk_stack_probe(void)
{
    int error = inspect(0, 0);
    if (error) return error;
    pthread_attr_t attr;
    if ((error = pthread_attr_init(&attr))) return error;
    if (!(error = pthread_attr_setstacksize(&attr, 128 * 1024)))
        error = pthread_attr_setguardsize(&attr, 8192);
    pthread_t thread;
    if (!error) error = pthread_create(&thread, &attr, worker, NULL);
    pthread_attr_destroy(&attr);
    if (error) return error;
    void *result = NULL;
    error = pthread_join(thread, &result);
    return error ? error : (int)(uintptr_t)result;
}
