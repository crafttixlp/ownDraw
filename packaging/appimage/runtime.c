/*
 * OwnDraw AppImage-Runtime (Typ-2-kompatibles Layout, ohne FUSE)
 *
 * Aufbau der AppImage-Datei:  [dieses statische ELF][SquashFS 4.0, gzip]
 * Das ELF traegt an Datei-Offset 8 die Bytes 'A','I',0x02 (wird nach dem Linken gepatcht).
 * Das SquashFS beginnt am Ende des ELF laut Section-Header (e_shoff + e_shnum*e_shentsize).
 *
 * Normaler Start: Payload einmalig nach $XDG_CACHE_HOME/appimage-run/<name>-<hash> entpacken
 * (Fallback ~/.cache/appimage-run, dann /tmp/appimage-run-<uid>), danach execv(<dir>/AppRun).
 *
 * Optionen: --appimage-extract [muster], --appimage-offset, --appimage-version, --appimage-help
 *
 * Bauen: gcc -static -O2 -o runtime runtime.c -lz
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <fnmatch.h>
#include <ftw.h>
#include <limits.h>
#include <signal.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <dirent.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/types.h>
#include <unistd.h>
#include <zlib.h>

#define RUNTIME_VERSION "1.0"
#define META_SIZE 8192
#define MAX_DEPTH 128

/* ------------------------------------------------------------------ Fehler */

static const char *prog = "AppImage";

static void die(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    fprintf(stderr, "%s: ", prog);
    vfprintf(stderr, fmt, ap);
    fputc('\n', stderr);
    va_end(ap);
    exit(1);
}

/* Fehler beim Lesen des SquashFS: Rueckgabe -1, Meldung einmal ausgeben */
static int fail(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    fprintf(stderr, "%s: SquashFS-Fehler: ", prog);
    vfprintf(stderr, fmt, ap);
    fputc('\n', stderr);
    va_end(ap);
    return -1;
}

/* ------------------------------------------------------------------ Image */

static const uint8_t *img;      /* gesamte Datei (mmap) */
static uint64_t img_size;
static uint64_t sq_off;         /* Offset des SquashFS in der Datei */
static uint64_t sq_size;        /* Bytes ab sq_off bis Dateiende */

/* Zeiger auf [off, off+len) relativ zum SquashFS-Anfang, NULL falls ausserhalb */
static const uint8_t *sq_at(uint64_t off, uint64_t len) {
    if (off > sq_size || len > sq_size - off) return NULL;
    return img + sq_off + off;
}

static uint16_t le16(const uint8_t *p) { return (uint16_t)(p[0] | (p[1] << 8)); }
static uint32_t le32(const uint8_t *p) {
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}
static uint64_t le64(const uint8_t *p) { return (uint64_t)le32(p) | ((uint64_t)le32(p + 4) << 32); }

/* ------------------------------------------------------------------ Superblock */

static struct {
    uint32_t inode_count, mod_time, block_size, frag_count;
    uint16_t compressor, block_log, flags;
    uint64_t root_inode, bytes_used, id_table, xattr_table, inode_table, dir_table, frag_table, export_table;
} sb;

static int read_superblock(void) {
    const uint8_t *p = sq_at(0, 96);
    if (!p) return fail("Payload zu kurz");
    if (le32(p) != 0x73717368) return fail("keine SquashFS-Signatur am Offset %llu", (unsigned long long)sq_off);
    sb.inode_count = le32(p + 4);
    sb.mod_time = le32(p + 8);
    sb.block_size = le32(p + 12);
    sb.frag_count = le32(p + 16);
    sb.compressor = le16(p + 20);
    sb.block_log = le16(p + 22);
    sb.flags = le16(p + 24);
    if (le16(p + 28) != 4 || le16(p + 30) != 0) return fail("nur SquashFS 4.0 wird unterstuetzt");
    sb.root_inode = le64(p + 32);
    sb.bytes_used = le64(p + 40);
    sb.id_table = le64(p + 48);
    sb.xattr_table = le64(p + 56);
    sb.inode_table = le64(p + 64);
    sb.dir_table = le64(p + 72);
    sb.frag_table = le64(p + 80);
    sb.export_table = le64(p + 88);
    if (sb.compressor != 1) return fail("Kompression %u nicht unterstuetzt (nur gzip)", sb.compressor);
    if (sb.block_size < 4096 || sb.block_size > (1u << 20) || (sb.block_size & (sb.block_size - 1)) ||
        sb.block_log > 20 || (1u << sb.block_log) != sb.block_size)
        return fail("ungueltige Blockgroesse %u", sb.block_size);
    if (sb.bytes_used > sq_size) return fail("Payload abgeschnitten (%llu von %llu Bytes)",
                                             (unsigned long long)sq_size, (unsigned long long)sb.bytes_used);
    if (sb.inode_table >= sb.dir_table || sb.dir_table > sb.bytes_used) return fail("ungueltige Tabellen-Offsets");
    return 0;
}

/* ------------------------------------------------------------------ Metadaten-Tabellen */

typedef struct {
    uint8_t *data;
    size_t len, cap;
    uint64_t *blk_disk;   /* Offset des Blocks relativ zum Tabellenanfang */
    size_t *blk_pos;      /* Position im dekomprimierten Puffer */
    size_t nblk, blkcap;
} mtable;

/* Liest einen Metadatenblock bei 'pos' und haengt ihn an t an; liefert Laenge auf Platte oder -1 */
static long read_meta_block(mtable *t, uint64_t pos) {
    const uint8_t *h = sq_at(pos, 2);
    if (!h) return -1;
    uint16_t hdr = le16(h);
    uint32_t size = hdr & 0x7fff;
    if (size == 0 || size > META_SIZE) return -1;
    const uint8_t *src = sq_at(pos + 2, size);
    if (!src) return -1;
    if (t->len + META_SIZE > t->cap) {
        size_t nc = t->cap ? t->cap * 2 : 65536;
        while (nc < t->len + META_SIZE) nc *= 2;
        uint8_t *nd = realloc(t->data, nc);
        if (!nd) return -1;
        t->data = nd;
        t->cap = nc;
    }
    uLongf out = META_SIZE;
    if (hdr & 0x8000) {
        memcpy(t->data + t->len, src, size);
        out = size;
    } else if (uncompress(t->data + t->len, &out, src, size) != Z_OK) {
        return -1;
    }
    t->len += out;
    return (long)(2 + size);
}

static int read_mtable(mtable *t, uint64_t start, uint64_t end, int strict) {
    memset(t, 0, sizeof *t);
    uint64_t pos = start;
    while (pos < end) {
        if (t->nblk == t->blkcap) {
            t->blkcap = t->blkcap ? t->blkcap * 2 : 256;
            t->blk_disk = realloc(t->blk_disk, t->blkcap * sizeof *t->blk_disk);
            t->blk_pos = realloc(t->blk_pos, t->blkcap * sizeof *t->blk_pos);
            if (!t->blk_disk || !t->blk_pos) return fail("kein Speicher");
        }
        size_t before = t->len;
        long n = read_meta_block(t, pos);
        if (n < 0) {
            if (strict || t->nblk == 0) return fail("defekter Metadatenblock bei %llu", (unsigned long long)pos);
            break; /* Ende der Tabelle erreicht (folgende Daten gehoeren zu anderen Tabellen) */
        }
        t->blk_disk[t->nblk] = pos - start;
        t->blk_pos[t->nblk] = before;
        t->nblk++;
        pos += (uint64_t)n;
    }
    return 0;
}

/* Wandelt (Block relativ zur Tabelle, Offset im Block) in Pufferposition */
static int mt_resolve(const mtable *t, uint64_t block, uint32_t off, size_t *out) {
    size_t lo = 0, hi = t->nblk;
    while (lo < hi) {
        size_t mid = (lo + hi) / 2;
        if (t->blk_disk[mid] < block) lo = mid + 1;
        else hi = mid;
    }
    if (lo >= t->nblk || t->blk_disk[lo] != block) return -1;
    size_t p = t->blk_pos[lo] + off;
    if (off >= META_SIZE || p >= t->len) return -1;
    *out = p;
    return 0;
}

static const uint8_t *mt_at(const mtable *t, size_t pos, size_t len) {
    if (pos > t->len || len > t->len - pos) return NULL;
    return t->data + pos;
}

static mtable inodes, dirs;

/* ------------------------------------------------------------------ Fragmente */

static uint8_t *frag_entries; /* frag_count * 16 Bytes */
static uint8_t *blockbuf, *fragbuf;
static long frag_cached = -1;
static uint32_t frag_cached_len;

static int read_frag_table(void) {
    if (sb.frag_count == 0) return 0;
    if (sb.frag_count > (1u << 26)) return fail("zu viele Fragmente");
    uint64_t need = (uint64_t)sb.frag_count * 16;
    uint32_t nidx = (uint32_t)((need + META_SIZE - 1) / META_SIZE);
    const uint8_t *idx = sq_at(sb.frag_table, (uint64_t)nidx * 8);
    if (!idx) return fail("Fragment-Index ausserhalb");
    mtable t;
    memset(&t, 0, sizeof t);
    for (uint32_t i = 0; i < nidx; i++) {
        size_t before = t.len;
        if (read_meta_block(&t, le64(idx + 8 * i)) < 0) return fail("defekte Fragment-Tabelle");
        if (i + 1 < nidx && t.len - before != META_SIZE) return fail("defekte Fragment-Tabelle");
    }
    if (t.len < need) return fail("Fragment-Tabelle zu kurz");
    frag_entries = t.data;
    return 0;
}

static int load_fragment(uint32_t index) {
    if (frag_cached == (long)index) return 0;
    if (index >= sb.frag_count) return fail("Fragment-Index %u ausserhalb", index);
    const uint8_t *e = frag_entries + 16 * (size_t)index;
    uint64_t start = le64(e);
    uint32_t w = le32(e + 8);
    uint32_t size = w & 0xffffff;
    const uint8_t *src = sq_at(start, size);
    if (!src || size == 0 || size > sb.block_size) return fail("Fragment %u ausserhalb", index);
    if (w & (1u << 24)) {
        memcpy(fragbuf, src, size);
        frag_cached_len = size;
    } else {
        uLongf out = sb.block_size;
        if (uncompress(fragbuf, &out, src, size) != Z_OK) return fail("Fragment %u nicht dekomprimierbar", index);
        frag_cached_len = (uint32_t)out;
    }
    frag_cached = index;
    return 0;
}

/* ------------------------------------------------------------------ Inodes */

enum { T_DIR = 1, T_FILE = 2, T_SYMLINK = 3, T_XDIR = 8, T_XFILE = 9, T_XSYMLINK = 10 };

typedef struct {
    uint16_t type, mode;
    uint32_t mtime;
    /* Verzeichnis */
    uint32_t dir_block, dir_size;
    uint16_t dir_offset;
    /* Datei */
    uint64_t blocks_start, file_size;
    uint32_t frag, frag_off, nblocks;
    size_t sizes_pos; /* Position der Blockgroessen-Liste in 'inodes' */
    /* Symlink */
    const char *target;
    uint32_t target_len;
} inode;

static int read_inode(uint64_t ref, inode *in) {
    size_t p;
    memset(in, 0, sizeof *in);
    if (mt_resolve(&inodes, ref >> 16, (uint32_t)(ref & 0xffff), &p) < 0)
        return fail("Inode-Referenz %llx ungueltig", (unsigned long long)ref);
    const uint8_t *h = mt_at(&inodes, p, 16);
    if (!h) return fail("Inode abgeschnitten");
    in->type = le16(h);
    in->mode = le16(h + 2);
    in->mtime = le32(h + 8);
    p += 16;
    const uint8_t *b;
    switch (in->type) {
    case T_DIR:
        if (!(b = mt_at(&inodes, p, 16))) return fail("Inode abgeschnitten");
        in->dir_block = le32(b);
        in->dir_size = le16(b + 8);
        in->dir_offset = le16(b + 10);
        break;
    case T_XDIR:
        if (!(b = mt_at(&inodes, p, 24))) return fail("Inode abgeschnitten");
        in->dir_size = le32(b + 4);
        in->dir_block = le32(b + 8);
        in->dir_offset = le16(b + 18);
        break;
    case T_FILE:
    case T_XFILE:
        if (in->type == T_FILE) {
            if (!(b = mt_at(&inodes, p, 16))) return fail("Inode abgeschnitten");
            in->blocks_start = le32(b);
            in->frag = le32(b + 4);
            in->frag_off = le32(b + 8);
            in->file_size = le32(b + 12);
            p += 16;
        } else {
            if (!(b = mt_at(&inodes, p, 40))) return fail("Inode abgeschnitten");
            in->blocks_start = le64(b);
            in->file_size = le64(b + 8);
            in->frag = le32(b + 28);
            in->frag_off = le32(b + 32);
            p += 40;
        }
        {
            uint64_t n = in->frag == 0xffffffffu ? (in->file_size + sb.block_size - 1) / sb.block_size
                                                 : in->file_size / sb.block_size;
            if (n > (1u << 28) || !mt_at(&inodes, p, n * 4)) return fail("Blockliste abgeschnitten");
            in->nblocks = (uint32_t)n;
            in->sizes_pos = p;
        }
        break;
    case T_SYMLINK:
    case T_XSYMLINK:
        if (!(b = mt_at(&inodes, p, 8))) return fail("Inode abgeschnitten");
        in->target_len = le32(b + 4);
        if (in->target_len == 0 || in->target_len >= PATH_MAX || !mt_at(&inodes, p + 8, in->target_len))
            return fail("Symlink-Ziel ungueltig");
        in->target = (const char *)inodes.data + p + 8;
        break;
    default:
        break; /* Geraete, FIFOs, Sockets: ignorieren */
    }
    return 0;
}

/* ------------------------------------------------------------------ Entpacken */

static const char *pattern;  /* nur bei --appimage-extract */
static int verbose;          /* Pfade ausgeben (wie die offizielle Runtime) */
static size_t root_len;      /* Laenge des Zielordner-Praefix im Pfad */

static int write_all(int fd, const uint8_t *p, size_t n) {
    while (n) {
        ssize_t w = write(fd, p, n);
        if (w < 0) {
            if (errno == EINTR) continue;
            return -1;
        }
        p += w;
        n -= (size_t)w;
    }
    return 0;
}

static void set_mtime(const char *path, uint32_t mtime) {
    struct timespec ts[2] = {{mtime, 0}, {mtime, 0}};
    utimensat(AT_FDCWD, path, ts, AT_SYMLINK_NOFOLLOW);
}

static int mkdir_p(char *path) {
    for (char *s = path + 1; *s; s++) {
        if (*s != '/') continue;
        *s = 0;
        if (mkdir(path, 0755) < 0 && errno != EEXIST) { *s = '/'; return -1; }
        *s = '/';
    }
    if (mkdir(path, 0755) < 0 && errno != EEXIST) return -1;
    return 0;
}

static int extract_file(const inode *in, const char *path) {
    unlink(path);
    int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC | O_NOFOLLOW, 0600);
    if (fd < 0) return fail("kann %s nicht anlegen: %s", path, strerror(errno));
    uint64_t pos = in->blocks_start, remaining = in->file_size;
    for (uint32_t i = 0; i < in->nblocks; i++) {
        uint32_t w = le32(inodes.data + in->sizes_pos + 4 * (size_t)i);
        uint32_t size = w & 0xffffff;
        uint32_t expect = remaining < sb.block_size ? (uint32_t)remaining : sb.block_size;
        if (size == 0) { /* Sparse-Block */
            if (lseek(fd, expect, SEEK_CUR) < 0) goto ioerr;
        } else {
            const uint8_t *src = sq_at(pos, size);
            if (!src || size > sb.block_size) { close(fd); return fail("Datenblock ausserhalb (%s)", path); }
            if (w & (1u << 24)) {
                if (size != expect) { close(fd); return fail("Blockgroesse falsch (%s)", path); }
                if (write_all(fd, src, size) < 0) goto ioerr;
            } else {
                uLongf out = sb.block_size;
                if (uncompress(blockbuf, &out, src, size) != Z_OK || out != expect) {
                    close(fd);
                    return fail("Datenblock defekt (%s)", path);
                }
                if (write_all(fd, blockbuf, out) < 0) goto ioerr;
            }
            pos += size;
        }
        remaining -= expect;
    }
    if (remaining) {
        if (in->frag == 0xffffffffu) { close(fd); return fail("Datei unvollstaendig (%s)", path); }
        if (load_fragment(in->frag) < 0) { close(fd); return -1; }
        if (in->frag_off > frag_cached_len || remaining > frag_cached_len - in->frag_off) {
            close(fd);
            return fail("Fragment-Bereich ungueltig (%s)", path);
        }
        if (write_all(fd, fragbuf + in->frag_off, (size_t)remaining) < 0) goto ioerr;
    }
    if (ftruncate(fd, (off_t)in->file_size) < 0) goto ioerr;
    if (fchmod(fd, in->mode & 0777) < 0) goto ioerr;
    if (close(fd) < 0) return fail("Schreibfehler %s: %s", path, strerror(errno));
    set_mtime(path, in->mtime);
    return 0;
ioerr:
    {
        int e = errno;
        close(fd);
        return fail("Schreibfehler %s: %s", path, strerror(e));
    }
}

static int matches(const char *path) {
    if (!pattern) return 1;
    const char *rel = path + root_len + 1;
    return fnmatch(pattern, rel, 0) == 0;
}

static int extract_dir(const inode *dir, char *path, size_t plen, int depth);

static int extract_entry(uint64_t ref, char *path, size_t plen, int depth) {
    inode in;
    if (read_inode(ref, &in) < 0) return -1;
    int want = matches(path);
    if (want && verbose) printf("%s\n", path);
    switch (in.type) {
    case T_DIR:
    case T_XDIR:
        if (!pattern) {
            if (mkdir(path, 0700) < 0 && errno != EEXIST) return fail("kann %s nicht anlegen: %s", path, strerror(errno));
        } else if (want && mkdir_p(path) < 0) {
            return fail("kann %s nicht anlegen: %s", path, strerror(errno));
        }
        if (extract_dir(&in, path, plen, depth + 1) < 0) return -1;
        if (want) {
            chmod(path, in.mode & 0777);
            set_mtime(path, in.mtime);
        }
        return 0;
    case T_FILE:
    case T_XFILE:
        if (!want) return 0;
        if (pattern) {
            char *slash = strrchr(path, '/');
            *slash = 0;
            int r = mkdir_p(path);
            *slash = '/';
            if (r < 0) return fail("kann Ordner fuer %s nicht anlegen", path);
        }
        return extract_file(&in, path);
    case T_SYMLINK:
    case T_XSYMLINK: {
        if (!want) return 0;
        if (pattern) {
            char *slash = strrchr(path, '/');
            *slash = 0;
            mkdir_p(path);
            *slash = '/';
        }
        char target[PATH_MAX];
        memcpy(target, in.target, in.target_len);
        target[in.target_len] = 0;
        if (strlen(target) != in.target_len) return fail("Symlink-Ziel enthaelt NUL");
        unlink(path);
        if (symlink(target, path) < 0) return fail("kann Symlink %s nicht anlegen: %s", path, strerror(errno));
        set_mtime(path, in.mtime);
        return 0;
    }
    default:
        return 0; /* Geraete, FIFOs, Sockets werden ignoriert */
    }
}

static int extract_dir(const inode *dir, char *path, size_t plen, int depth) {
    if (depth > MAX_DEPTH) return fail("Verzeichnisse zu tief verschachtelt");
    if (dir->dir_size <= 3) return 0;
    size_t p;
    if (mt_resolve(&dirs, dir->dir_block, dir->dir_offset, &p) < 0) return fail("Verzeichnis-Referenz ungueltig");
    size_t end = p + dir->dir_size - 3;
    if (end > dirs.len) return fail("Verzeichnis abgeschnitten");
    while (p < end) {
        const uint8_t *h = mt_at(&dirs, p, 12);
        if (!h || p + 12 > end) return fail("Verzeichniskopf defekt");
        uint32_t count = le32(h) + 1;
        uint32_t start = le32(h + 4);
        p += 12;
        if (count > 256) return fail("Verzeichniskopf defekt");
        for (uint32_t i = 0; i < count; i++) {
            const uint8_t *e = mt_at(&dirs, p, 8);
            if (!e || p + 8 > end) return fail("Verzeichniseintrag defekt");
            uint16_t off = le16(e);
            uint32_t nlen = (uint32_t)le16(e + 6) + 1;
            const uint8_t *name = mt_at(&dirs, p + 8, nlen);
            if (!name || p + 8 + nlen > end) return fail("Verzeichniseintrag defekt");
            p += 8 + nlen;
            /* Namen pruefen: kein '/', kein NUL, nicht . oder .. */
            if (memchr(name, '/', nlen) || memchr(name, 0, nlen) || (nlen == 1 && name[0] == '.') ||
                (nlen == 2 && name[0] == '.' && name[1] == '.'))
                return fail("ungueltiger Dateiname im Archiv");
            if (plen + 1 + nlen >= PATH_MAX) return fail("Pfad zu lang");
            path[plen] = '/';
            memcpy(path + plen + 1, name, nlen);
            path[plen + 1 + nlen] = 0;
            if (extract_entry(((uint64_t)start << 16) | off, path, plen + 1 + nlen, depth) < 0) return -1;
            path[plen] = 0;
        }
    }
    return 0;
}

static int open_squashfs(void) {
    if (read_superblock() < 0) return -1;
    if (read_mtable(&inodes, sb.inode_table, sb.dir_table, 1) < 0) return -1;
    /* Ende der Verzeichnistabelle: naechste Tabelle dahinter */
    uint64_t end = sb.bytes_used;
    uint64_t cand[] = {sb.frag_table, sb.export_table, sb.id_table, sb.xattr_table};
    for (int i = 0; i < 4; i++)
        if (cand[i] > sb.dir_table && cand[i] < end) end = cand[i];
    if (read_mtable(&dirs, sb.dir_table, end, 0) < 0) return -1;
    blockbuf = malloc(sb.block_size);
    fragbuf = malloc(sb.block_size);
    if (!blockbuf || !fragbuf) return fail("kein Speicher");
    if (read_frag_table() < 0) return -1;
    return 0;
}

/* Entpackt das komplette Archiv nach 'dest' (Ordner wird angelegt bzw. wiederverwendet) */
static int extract_all(const char *dest) {
    char path[PATH_MAX];
    size_t n = strlen(dest);
    if (n >= PATH_MAX - 1) return fail("Pfad zu lang");
    memcpy(path, dest, n + 1);
    while (n > 1 && path[n - 1] == '/') path[--n] = 0;
    root_len = n;
    inode root;
    if (read_inode(sb.root_inode, &root) < 0) return -1;
    if (root.type != T_DIR && root.type != T_XDIR) return fail("Wurzel ist kein Verzeichnis");
    if (mkdir(path, 0700) < 0 && errno != EEXIST) return fail("kann %s nicht anlegen: %s", path, strerror(errno));
    if (extract_dir(&root, path, n, 0) < 0) return -1;
    if (!pattern) chmod(path, (root.mode & 0777) | 0700);
    return 0;
}

/* ------------------------------------------------------------------ Hilfen */

static int rm_cb(const char *p, const struct stat *st, int flag, struct FTW *f) {
    (void)st; (void)f;
    if (flag == FTW_DP || flag == FTW_D) return rmdir(p) < 0 ? 0 : 0;
    unlink(p);
    return 0;
}
static int chmod_cb(const char *p, const struct stat *st, int flag, struct FTW *f) {
    (void)f;
    if (flag == FTW_D || flag == FTW_DNR) chmod(p, (st->st_mode & 0777) | 0700);
    return 0;
}
static void rm_rf(const char *path) {
    nftw(path, chmod_cb, 32, FTW_PHYS);
    nftw(path, rm_cb, 32, FTW_PHYS | FTW_DEPTH);
}

static uint64_t fnv(uint64_t h, const void *data, size_t n) {
    const uint8_t *p = data;
    for (size_t i = 0; i < n; i++) {
        h ^= p[i];
        h *= 0x100000001b3ULL;
    }
    return h;
}

static int dir_writable(const char *d) {
    char tmp[PATH_MAX];
    if (snprintf(tmp, sizeof tmp, "%s", d) >= (int)sizeof tmp) return 0;
    if (mkdir_p(tmp) < 0) return 0;
    return access(d, W_OK | X_OK) == 0;
}

/* Verwaiste temporaere Ordner frueherer, abgebrochener Starts entfernen */
static void clean_stale(const char *base) {
    DIR *d = opendir(base);
    if (!d) return;
    struct dirent *e;
    char p[PATH_MAX];
    while ((e = readdir(d))) {
        const char *t = strstr(e->d_name, ".tmp-");
        if (e->d_name[0] != '.' || !t) continue;
        long pid = strtol(t + 5, NULL, 10);
        if (pid <= 0 || pid == getpid()) continue;
        if (kill((pid_t)pid, 0) == 0 || errno != ESRCH) continue;
        if (snprintf(p, sizeof p, "%s/%s", base, e->d_name) < (int)sizeof p) rm_rf(p);
    }
    closedir(d);
}

static int is_complete(const char *dir) {
    char p[PATH_MAX];
    struct stat st;
    snprintf(p, sizeof p, "%s/.complete", dir);
    return stat(p, &st) == 0;
}

static void usage(void) {
    printf("OwnDraw AppImage-Runtime " RUNTIME_VERSION "\n\n"
           "Aufruf: %s [OPTION|Argumente der App]\n\n"
           "  --appimage-extract [muster]  Inhalt nach ./squashfs-root entpacken (optional nur passende Pfade)\n"
           "  --appimage-offset            Offset des SquashFS in der Datei ausgeben\n"
           "  --appimage-version           Version der Runtime ausgeben\n"
           "  --appimage-help              diese Hilfe\n\n"
           "Ohne Option wird der Inhalt einmalig nach $XDG_CACHE_HOME/appimage-run/ entpackt und AppRun gestartet.\n",
           prog);
}

/* ------------------------------------------------------------------ main */

int main(int argc, char **argv) {
    static char self[PATH_MAX];
    if (!realpath("/proc/self/exe", self)) die("kann eigenen Pfad nicht bestimmen: %s", strerror(errno));
    const char *bn = strrchr(self, '/');
    bn = bn ? bn + 1 : self;
    prog = bn;

    int fd = open(self, O_RDONLY | O_CLOEXEC);
    if (fd < 0) die("kann %s nicht oeffnen: %s", self, strerror(errno));
    struct stat st;
    if (fstat(fd, &st) < 0) die("fstat: %s", strerror(errno));
    img_size = (uint64_t)st.st_size;
    if (img_size < 64) die("Datei zu kurz");
    void *m = mmap(NULL, img_size, PROT_READ, MAP_PRIVATE, fd, 0);
    if (m == MAP_FAILED) die("mmap: %s", strerror(errno));
    close(fd);
    img = m;

    /* ELF64-Kopf -> Ende des ELF = Anfang des SquashFS */
    if (memcmp(img, "\177ELF", 4) != 0 || img[4] != 2 || img[5] != 1) die("kein ELF64 (little endian)");
    uint64_t shoff = le64(img + 0x28);
    uint16_t shentsize = le16(img + 0x3a), shnum = le16(img + 0x3c);
    sq_off = shoff + (uint64_t)shentsize * shnum;
    if (sq_off > img_size) die("ELF-Kopf ungueltig");
    sq_size = img_size - sq_off;

    if (argc > 1 && strncmp(argv[1], "--appimage-", 11) == 0) {
        const char *opt = argv[1] + 11;
        if (!strcmp(opt, "offset")) {
            printf("%llu\n", (unsigned long long)sq_off);
            return 0;
        }
        if (!strcmp(opt, "version")) {
            printf("OwnDraw AppImage-Runtime " RUNTIME_VERSION " (Typ-2-Layout, SquashFS/gzip, ohne FUSE)\n");
            return 0;
        }
        if (!strcmp(opt, "help")) {
            usage();
            return 0;
        }
        if (!strcmp(opt, "extract")) {
            if (argc > 2) pattern = argv[2];
            verbose = 1;
            if (open_squashfs() < 0) return 1;
            if (extract_all("squashfs-root") < 0) return 1;
            return 0;
        }
        fprintf(stderr, "%s: unbekannte Option %s (siehe --appimage-help)\n", prog, argv[1]);
        return 1;
    }

    /* ---- normaler Start: Cache-Ordner bestimmen */
    if (sq_size < 96) die("kein Payload angehaengt");
    uint64_t h = 0xcbf29ce484222325ULL;
    uint64_t v = sq_size;
    h = fnv(h, &v, sizeof v);
    v = (uint64_t)st.st_mtime;
    h = fnv(h, &v, sizeof v);
    size_t head = sq_size < 65536 ? (size_t)sq_size : 65536;
    h = fnv(h, img + sq_off, head);
    h = fnv(h, img + img_size - head, head);

    char name[256];
    size_t nl = strlen(bn);
    if (nl > 9 && strcasecmp(bn + nl - 9, ".AppImage") == 0) nl -= 9;
    if (nl > 200) nl = 200;
    for (size_t i = 0; i < nl; i++) {
        char c = bn[i];
        name[i] = ((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '.' ||
                   c == '_' || c == '-') ? c : '_';
    }
    name[nl] = 0;
    if (nl == 0 || name[0] == '.') name[0] = '_';

    char base[PATH_MAX], dir[PATH_MAX], tmp[PATH_MAX];
    const char *xdg = getenv("XDG_CACHE_HOME"), *home = getenv("HOME");
    int ok = 0;
    if (xdg && xdg[0] == '/' && snprintf(base, sizeof base, "%s/appimage-run", xdg) < (int)sizeof base - 300)
        ok = dir_writable(base);
    if (!ok && home && home[0] == '/' && snprintf(base, sizeof base, "%s/.cache/appimage-run", home) < (int)sizeof base - 300)
        ok = dir_writable(base);
    if (!ok) {
        snprintf(base, sizeof base, "/tmp/appimage-run-%u", (unsigned)getuid());
        if (mkdir(base, 0700) < 0 && errno != EEXIST) die("kein beschreibbarer Cache-Ordner");
        struct stat bs;
        if (lstat(base, &bs) < 0 || !S_ISDIR(bs.st_mode) || bs.st_uid != getuid()) die("%s gehoert nicht dem Nutzer", base);
    }
    snprintf(dir, sizeof dir, "%s/%s-%016llx", base, name, (unsigned long long)h);

    if (!is_complete(dir)) {
        clean_stale(base);
        snprintf(tmp, sizeof tmp, "%s/.%s-%016llx.tmp-%ld", base, name, (unsigned long long)h, (long)getpid());
        rm_rf(tmp);
        if (open_squashfs() < 0 || extract_all(tmp) < 0) {
            rm_rf(tmp);
            die("Entpacken fehlgeschlagen");
        }
        char marker[PATH_MAX];
        snprintf(marker, sizeof marker, "%s/.complete", tmp);
        int mfd = open(marker, O_WRONLY | O_CREAT | O_TRUNC, 0644);
        if (mfd < 0) { rm_rf(tmp); die("kann %s nicht schreiben", marker); }
        dprintf(mfd, "%s\n", self);
        close(mfd);
        if (rename(tmp, dir) < 0) {
            if (is_complete(dir)) {
                rm_rf(tmp); /* ein paralleler Start war schneller: dessen Ordner nehmen */
            } else {
                /* unvollstaendiger Rest (z. B. manuell angelegt): beiseiteschieben und erneut versuchen */
                char trash[PATH_MAX];
                snprintf(trash, sizeof trash, "%s/.%s-%016llx.tmp-old%ld", base, name, (unsigned long long)h, (long)getpid());
                if (rename(dir, trash) == 0) rm_rf(trash);
                if (rename(tmp, dir) < 0 && !is_complete(dir)) {
                    rm_rf(tmp);
                    die("kann %s nicht anlegen: %s", dir, strerror(errno));
                }
                rm_rf(tmp);
            }
        }
    }

    char apprun[PATH_MAX], owd[PATH_MAX];
    snprintf(apprun, sizeof apprun, "%s/AppRun", dir);
    setenv("APPIMAGE", self, 1);
    setenv("APPDIR", dir, 1);
    setenv("ARGV0", argv[0] ? argv[0] : self, 1);
    if (getcwd(owd, sizeof owd)) setenv("OWD", owd, 1);
    munmap((void *)img, img_size);
    execv(apprun, argv);
    die("kann %s nicht starten: %s", apprun, strerror(errno));
    return 127;
}
