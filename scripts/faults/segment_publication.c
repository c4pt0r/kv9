/* Linux/glibc syscall-result injection for one explicitly selected local WAL.
 * Loaded only into the owned probe process, never Cargo or a database service.
 * A recorded cut is required by the runner; a silent unused hook cannot pass.
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static _Atomic int fired;
static _Atomic int published;
static _Atomic int removed;
static _Atomic int captured;

static int mode(const char *value) {
    const char *cut = getenv("KV9_PUBLICATION_CUT");
    return cut && strcmp(cut, value) == 0;
}

static int selected(const char *path) {
    const char *target = getenv("KV9_PUBLICATION_TARGET");
    return target && strcmp(target, path) == 0;
}

static int segment(const char *path) {
    const char *directory = getenv("KV9_PUBLICATION_SEGMENTS");
    return directory && strncmp(path, directory, strlen(directory)) == 0 &&
           path[strlen(directory)] == '/';
}

static void evidence(const char *event, int real_succeeded) {
    int saved = errno;
    const char *path = getenv("KV9_PUBLICATION_TRACE");
    if (!path) _exit(120);
    int fd = open(path, O_CREAT | O_APPEND | O_WRONLY, 0600);
    if (fd < 0) _exit(121);
    char line[256];
    int count = snprintf(line, sizeof(line), "{\"event\":\"%s\",\"real_succeeded\":%s}\n",
                         event, real_succeeded ? "true" : "false");
    if (count < 0 || (size_t)count >= sizeof(line) || write(fd, line, (size_t)count) != count) _exit(122);
    if (close(fd) != 0) _exit(123);
    errno = saved;
}

static int inject(const char *event, int real_succeeded) {
    if (atomic_exchange(&fired, 1)) _exit(124);
    evidence(event, real_succeeded);
    errno = EIO;
    return -1;
}

static void capture_old(const char *path) {
    if (atomic_exchange(&captured, 1)) return;
    const char *destination = getenv("KV9_PUBLICATION_OLD_ROOT");
    if (!destination) _exit(128);
    int input = open(path, O_RDONLY);
    int output = open(destination, O_CREAT | O_EXCL | O_WRONLY, 0600);
    if (input < 0 || output < 0) _exit(129);
    char buffer[4096];
    ssize_t count;
    while ((count = read(input, buffer, sizeof(buffer))) > 0) {
        if (write(output, buffer, (size_t)count) != count) _exit(130);
    }
    if (count < 0 || close(input) != 0 || close(output) != 0) _exit(131);
}

int rename(const char *old, const char *next) {
    int (*real_rename)(const char *, const char *) = dlsym(RTLD_NEXT, "rename");
    if (!real_rename) _exit(125);
    int target = selected(next);
    if (target) capture_old(next);
    if (target && !fired && mode("rename_before")) return inject("rename_before", 0);
    int result = real_rename(old, next);
    if (target && result == 0) {
        published = 1;
        evidence("rename_succeeded", 1);
        if (!fired && mode("rename_after")) return inject("rename_after", 1);
    }
    return result;
}

int fsync(int fd) {
    int (*real_fsync)(int) = dlsym(RTLD_NEXT, "fsync");
    if (!real_fsync) _exit(126);
    char proc[64], path[PATH_MAX + 1];
    struct stat info;
    snprintf(proc, sizeof(proc), "/proc/self/fd/%d", fd);
    ssize_t size = readlink(proc, path, sizeof(path) - 1);
    int directory = fstat(fd, &info) == 0 && S_ISDIR(info.st_mode);
    if (size < 0 || fired) return real_fsync(fd);
    path[size] = 0;
    if (!directory) {
        if (!segment(path) || !(mode("segment_sync_before") || mode("segment_sync_after"))) return real_fsync(fd);
        const char *target = getenv("KV9_PUBLICATION_TARGET");
        if (!target) _exit(132);
        capture_old(target);
        if (mode("segment_sync_before")) return inject("segment_sync_before", 0);
        int result = real_fsync(fd);
        if (result == 0) return inject("segment_sync_after", 1);
        return result;
    }
    const char *parent = getenv("KV9_PUBLICATION_PARENT");
    int publication = published && parent && strcmp(parent, path) == 0;
    int deletion = removed && segment(path);
    if (publication && mode("dirsync_before")) return inject("dirsync_before", 0);
    if (deletion && mode("unlink_dirsync_before")) return inject("unlink_dirsync_before", 0);
    int result = real_fsync(fd);
    if (result == 0 && publication) {
        evidence("publication_dirsync_succeeded", 1);
        if (mode("dirsync_after")) return inject("dirsync_after", 1);
    }
    if (result == 0 && deletion && mode("unlink_dirsync_after")) return inject("unlink_dirsync_after", 1);
    return result;
}

int unlink(const char *path) {
    int (*real_unlink)(const char *) = dlsym(RTLD_NEXT, "unlink");
    if (!real_unlink) _exit(127);
    int target = published && segment(path);
    if (target && !fired && mode("unlink_before")) return inject("unlink_before", 0);
    int result = real_unlink(path);
    if (target && result == 0) {
        removed = 1;
        evidence("unlink_succeeded", 1);
        if (!fired && mode("unlink_after")) return inject("unlink_after", 1);
    }
    return result;
}
