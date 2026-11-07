/* isocap - isochronous IN capture from a USB audio-streaming endpoint via usbfs.
 *
 * Why C: the kernel on the RG35XXSP has no snd-usb-audio, so we read the FM-1's
 * capture endpoint ourselves. Iso URB handling wants steady timing and a variable
 * length struct; doing it here keeps it out of Python's GIL and ctypes layout guessing.
 *
 *   isocap <devnode> <iface> <alt> <ep> <maxpkt> [frames_per_urb=16] [nurbs=8]
 *          [seconds=0 (0 = until SIGINT/SIGTERM)] [out=- (stdout)]
 *
 * Raw payload bytes (only each packet's actual_length) go to `out`. Statistics go to
 * stderr on exit as `key=value` lines. Exit status 0 on clean stop, 1 on setup error.
 * Always sets alt 0 and releases the interface before exiting.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <time.h>
#include <unistd.h>
#include <linux/usbdevice_fs.h>

static volatile sig_atomic_t g_stop = 0;
static void on_sig(int s) { (void)s; g_stop = 1; }

static double now_s(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec / 1e9;
}

static int write_all(int fd, const unsigned char *p, size_t n) {
    while (n) {
        ssize_t w = write(fd, p, n);
        if (w < 0) {
            if (errno == EINTR) continue;
            return -1;
        }
        p += w; n -= (size_t)w;
    }
    return 0;
}

int main(int argc, char **argv) {
    if (argc < 6) {
        fprintf(stderr, "usage: %s devnode iface alt ep maxpkt [frames_per_urb nurbs seconds out]\n", argv[0]);
        return 1;
    }
    const char *dev = argv[1];
    unsigned iface = (unsigned)atoi(argv[2]), alt = (unsigned)atoi(argv[3]);
    unsigned ep = (unsigned)strtoul(argv[4], NULL, 0);
    int maxpkt = atoi(argv[5]);
    int nframes = argc > 6 ? atoi(argv[6]) : 16;
    int nurbs = argc > 7 ? atoi(argv[7]) : 8;
    double seconds = argc > 8 ? atof(argv[8]) : 0;
    const char *outpath = argc > 9 ? argv[9] : "-";
    if (maxpkt <= 0 || nframes <= 0 || nurbs <= 0 || nurbs > 64) {
        fprintf(stderr, "bad numeric argument\n");
        return 1;
    }

    int out = 1;
    if (strcmp(outpath, "-") != 0) {
        out = open(outpath, O_WRONLY | O_CREAT | O_TRUNC, 0644);
        if (out < 0) { perror("open out"); return 1; }
    }
    int fd = open(dev, O_RDWR);
    if (fd < 0) { perror("open device"); return 1; }

    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = on_sig;
    sigaction(SIGINT, &sa, NULL);
    sigaction(SIGTERM, &sa, NULL);
    sigaction(SIGHUP, &sa, NULL);
    signal(SIGPIPE, SIG_IGN);

    if (ioctl(fd, USBDEVFS_CLAIMINTERFACE, &iface) < 0) { perror("claim interface"); return 1; }
    struct usbdevfs_setinterface si = { iface, alt };
    if (ioctl(fd, USBDEVFS_SETINTERFACE, &si) < 0) {
        perror("set alt setting");
        ioctl(fd, USBDEVFS_RELEASEINTERFACE, &iface);
        return 1;
    }

    size_t urb_sz = sizeof(struct usbdevfs_urb) + (size_t)nframes * sizeof(struct usbdevfs_iso_packet_desc);
    size_t buf_sz = (size_t)nframes * (size_t)maxpkt;
    struct usbdevfs_urb **urbs = calloc((size_t)nurbs, sizeof *urbs);
    unsigned char **bufs = calloc((size_t)nurbs, sizeof *bufs);
    int submitted = 0, rc = 0;
    long long packets = 0, empty = 0, bad = 0, bytes = 0, resubmit_fail = 0, urbs_done = 0;
    int first_bad = 0, min_len = 1 << 30, max_len = 0;

    for (int i = 0; i < nurbs; i++) {
        urbs[i] = calloc(1, urb_sz);
        bufs[i] = calloc(1, buf_sz);
        if (!urbs[i] || !bufs[i]) { fprintf(stderr, "out of memory\n"); rc = 1; goto done; }
        urbs[i]->type = USBDEVFS_URB_TYPE_ISO;
        urbs[i]->endpoint = (unsigned char)ep;
        urbs[i]->flags = USBDEVFS_URB_ISO_ASAP;
        urbs[i]->buffer = bufs[i];
        urbs[i]->buffer_length = (int)buf_sz;
        urbs[i]->number_of_packets = nframes;
        for (int k = 0; k < nframes; k++) urbs[i]->iso_frame_desc[k].length = (unsigned)maxpkt;
        if (ioctl(fd, USBDEVFS_SUBMITURB, urbs[i]) < 0) {
            perror("submit urb");
            rc = 1;
            goto done;
        }
        submitted++;
    }

    double t0 = now_s();
    struct pollfd pfd = { fd, POLLOUT, 0 };
    while (!g_stop && (seconds <= 0 || now_s() - t0 < seconds)) {
        int pr = poll(&pfd, 1, 100);
        if (pr < 0 && errno != EINTR) { perror("poll"); rc = 1; break; }
        if (pr <= 0) continue;
        for (;;) {
            struct usbdevfs_urb *u = NULL;
            if (ioctl(fd, USBDEVFS_REAPURBNDELAY, &u) < 0) {
                if (errno == EAGAIN) break;
                if (errno == ENODEV) { fprintf(stderr, "device disconnected\n"); g_stop = 1; rc = 2; break; }
                perror("reap"); g_stop = 1; rc = 1; break;
            }
            urbs_done++;
            unsigned char *base = u->buffer;
            for (int k = 0; k < u->number_of_packets; k++) {
                struct usbdevfs_iso_packet_desc *d = &u->iso_frame_desc[k];
                packets++;
                if (d->status != 0) { bad++; if (!first_bad) first_bad = (int)d->status; }
                if (d->actual_length == 0) { empty++; continue; }
                if ((int)d->actual_length < min_len) min_len = (int)d->actual_length;
                if ((int)d->actual_length > max_len) max_len = (int)d->actual_length;
                if (write_all(out, base + (size_t)k * (size_t)maxpkt, d->actual_length) < 0) {
                    fprintf(stderr, "output closed\n"); g_stop = 1; break;
                }
                bytes += d->actual_length;
                d->actual_length = 0;
                d->status = 0;
            }
            if (g_stop) break;
            u->start_frame = 0;
            if (ioctl(fd, USBDEVFS_SUBMITURB, u) < 0) { resubmit_fail++; submitted--; }
        }
    }

done:
    for (int i = 0; i < nurbs; i++)
        if (urbs[i] && submitted > 0) ioctl(fd, USBDEVFS_DISCARDURB, urbs[i]);
    for (int i = 0; i < nurbs; i++) {                       /* drain discarded URBs */
        struct usbdevfs_urb *u = NULL;
        if (ioctl(fd, USBDEVFS_REAPURBNDELAY, &u) < 0 && errno == EAGAIN) break;
    }
    si.altsetting = 0;
    ioctl(fd, USBDEVFS_SETINTERFACE, &si);                  /* stop streaming */
    ioctl(fd, USBDEVFS_RELEASEINTERFACE, &iface);
    close(fd);
    if (out != 1) close(out);
    fprintf(stderr, "packets=%lld\nempty=%lld\nbad_status=%lld\nfirst_bad_status=%d\n"
                    "bytes=%lld\nmin_len=%d\nmax_len=%d\nurbs=%lld\nresubmit_fail=%lld\n",
            packets, empty, bad, first_bad, bytes, min_len == (1 << 30) ? 0 : min_len, max_len,
            urbs_done, resubmit_fail);
    return rc;
}
