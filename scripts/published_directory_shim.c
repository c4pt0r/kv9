/* Process-local Linux/glibc fsync observation and finite EIO cuts.
 * No library/server fault hooks. Only the explicitly owned probe preloads this.
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

static int trace_fd = -1, fired, skipped;
static unsigned sequence;
static char phase_name[64] = "unstarted";
static const char *mode, *successor, *parent, *topology, *skip_ancestor;

static void die(int code) { _exit(code); }
static const char *setting(const char *name) {
    const char *s = getenv(name);
    if (!s || !*s || strlen(s) >= PATH_MAX || strpbrk(s, "\"\\\n\r\t")) die(120);
    return s;
}
static uint64_t now(clockid_t clock) {
    struct timespec t;
    if (clock_gettime(clock, &t)) die(121);
    return (uint64_t)t.tv_sec * 1000000000ULL + (uint64_t)t.tv_nsec;
}
static void record(const char *event, const char *path, const struct stat *info,
                   int real_called, int real_result, int returned, int error,
                   int injected, int omitted) {
    int saved = errno;
    if (trace_fd < 0 || ++sequence > 4096) die(122);
    char line[8192];
    int n = snprintf(line, sizeof(line),
        "{\"seq\":%u,\"pid\":%ld,\"phase\":\"%s\",\"event\":\"%s\","
        "\"path\":\"%s\",\"device\":%llu,\"inode\":%llu,\"mode\":%u,"
        "\"real_called\":%s,\"real_result\":%d,\"returned\":%d,\"errno\":%d,"
        "\"injected\":%s,\"omitted\":%s,\"fault_count\":%d,\"skip_count\":%d,"
        "\"monotonic_ns\":%llu,\"unix_ns\":%llu}\n",
        sequence, (long)getpid(), phase_name, event, path,
        info ? (unsigned long long)info->st_dev : 0ULL,
        info ? (unsigned long long)info->st_ino : 0ULL,
        info ? (unsigned)info->st_mode : 0U,
        real_called ? "true" : "false", real_result, returned, error,
        injected ? "true" : "false", omitted ? "true" : "false", fired, skipped,
        (unsigned long long)now(CLOCK_MONOTONIC), (unsigned long long)now(CLOCK_REALTIME));
    if (n < 0 || (size_t)n >= sizeof(line)) die(123);
    if (write(trace_fd, line, (size_t)n) != n) die(124);
    errno = saved;
}

int kv9_published_directory_phase(const char *phase) {
    if (!phase || !*phase || strlen(phase) >= sizeof(phase_name) ||
        strspn(phase, "abcdefghijklmnopqrstuvwxyz0123456789-") != strlen(phase)) die(125);
    if (trace_fd < 0) {
        mode = setting("KV9_PD_MODE");
        successor = setting("KV9_PD_SUCCESSOR");
        parent = setting("KV9_PD_PARENT");
        topology = setting("KV9_PD_TOPOLOGY");
        skip_ancestor = setting("KV9_PD_SKIP_ANCESTOR");
        trace_fd = open(setting("KV9_PD_TRACE"), O_CREAT | O_EXCL | O_WRONLY | O_CLOEXEC | O_NOFOLLOW, 0600);
        if (trace_fd < 0) die(126);
    }
    strcpy(phase_name, phase);
    record("phase", "", NULL, 0, 0, 0, 0, 0, 0);
    return 0x504431;
}

int fsync(int fd) {
    int (*real_fsync)(int) = dlsym(RTLD_NEXT, "fsync");
    if (!real_fsync) die(127);
    if (trace_fd < 0) return real_fsync(fd);
    char proc[64], path[PATH_MAX + 1];
    struct stat info;
    snprintf(proc, sizeof(proc), "/proc/self/fd/%d", fd);
    ssize_t size = readlink(proc, path, sizeof(path)-1);
    if (size < 0 || fstat(fd, &info)) die(128);
    path[size] = 0;
    if (strpbrk(path, "\"\\\n\r\t")) die(129);
    int directory = S_ISDIR(info.st_mode);
    int selected = strcmp(phase_name, "rotate-1") == 0 && !fired &&
        ((!directory && strcmp(path, successor) == 0 && strncmp(mode, "successor_", 10) == 0) ||
         (directory && strcmp(path, parent) == 0 && strncmp(mode, "parent_", 7) == 0));
    if (!skipped && directory &&
        ((strcmp(mode, "skip_ancestor") == 0 && strcmp(phase_name, "create") == 0 && strcmp(path, skip_ancestor) == 0) ||
         (strcmp(mode, "skip_successor_parent") == 0 && strcmp(phase_name, "rotate-1") == 0 && strcmp(path, parent) == 0))) {
        skipped = 1;
        record("fsync", path, &info, 0, 0, 0, 0, 0, 1);
        return 0;
    }
    int before = selected && (strcmp(mode, "successor_before") == 0 || strcmp(mode, "parent_before") == 0);
    if (before) {
        fired = 1;
        record("fsync", path, &info, 0, 0, -1, EIO, 1, 0);
        errno = EIO;
        return -1;
    }
    int result = real_fsync(fd), error = result ? errno : 0;
    int after = selected && result == 0 && (strcmp(mode, "successor_after") == 0 || strcmp(mode, "parent_after") == 0);
    if (after) {
        fired = 1;
        record("fsync", path, &info, 1, result, -1, EIO, 1, 0);
        errno = EIO;
        return -1;
    }
    record("fsync", path, &info, 1, result, result, error, 0, 0);
    if (result) errno = error;
    return result;
}

int rename(const char *old, const char *next) {
    int (*real_rename)(const char *, const char *) = dlsym(RTLD_NEXT, "rename");
    if (!real_rename) die(130);
    int result = real_rename(old, next), error = result ? errno : 0;
    if (trace_fd >= 0 && strcmp(next, topology) == 0) {
        struct stat info;
        if (result == 0 && stat(next, &info)) die(131);
        record("rename", next, result ? NULL : &info, 1, result, result, error, 0, 0);
    }
    if (result) errno = error;
    return result;
}

__attribute__((destructor)) static void finished(void) {
    if (trace_fd >= 0) {
        record("end", "", NULL, 0, 0, 0, 0, 0, 0);
        if (close(trace_fd)) die(132);
    }
}
