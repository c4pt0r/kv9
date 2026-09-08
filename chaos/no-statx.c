/* Test-container compatibility for Chaos Mesh 2.8.4's old FUSE server.
 * Linux 6.17 can send FUSE_STATX (52), which terminates that server. Expose
 * the normal ENOSYS fallback to this process tree only. Writes and fsync are
 * untouched; actual EIO/ENOSPC must still come from an IOChaos WRITE rule.
 * This is not a security sandbox and is never part of the kv9 binary.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <linux/filter.h>
#include <linux/seccomp.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

int main(int argc, char **argv) {
    struct sock_filter filter[] = {
        BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, nr)),
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_statx, 0, 1),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ERRNO | ENOSYS),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
    };
    struct sock_fprog program = { .len = sizeof(filter) / sizeof(filter[0]), .filter = filter };
    if (argc < 2 || prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) ||
        prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &program)) {
        perror("install statx compatibility filter");
        return 1;
    }
    if (!strcmp(argv[1], "--check")) {
        struct statx extended;
        struct stat basic;
        if (syscall(SYS_statx, AT_FDCWD, "/", 0, STATX_BASIC_STATS, &extended) != -1 ||
            errno != ENOSYS || stat("/", &basic) != 0 || !S_ISDIR(basic.st_mode)) {
            fputs("FAIL: statx fallback compatibility check\n", stderr);
            return 1;
        }
        puts("PASS: statx returns ENOSYS and ordinary metadata remains available");
        return 0;
    }
    execvp(argv[1], argv + 1);
    perror("exec with statx compatibility");
    return 1;
}
