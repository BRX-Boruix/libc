# libc

BORUIX's C standard library. Implemented in Rust, exporting standard C interfaces, built on top of [`libsys`](https://github.com/BRX-Boruix/libsys).

[简体中文](README.md)

## Implemented

| Area | Contents |
| --- | --- |
| Memory | `malloc` / `free` / `realloc` / `calloc` / `posix_memalign` / `aligned_alloc` |
| Strings and memory blocks | `memcpy` / `strlen` / `strcmp` / `strstr` / `strtok_r` and others |
| Formatted output | `printf` / `snprintf` / `fprintf` / `vprintf` with a full format engine (`%f` / `%e` / `%g`) |
| Formatted input | The `fscanf` family |
| File streams | `fopen` / `fread` / `fwrite` / `fgets` / `fputs` / `fflush` and others |
| Numeric conversion and sorting | `strtol` / `strtod` / `atoi` / `qsort` / `bsearch` |
| Character classification | `isalpha` / `tolower` / `toupper` and others |
| Syscall wrappers | `open` / `read` / `write` / `lseek` / `unlink` / `chdir` / `getcwd` / `isatty` |
| Process and time | `exit` / `getpid` / `kill` / `waitpid` / `time` / `nanosleep` |
| Wide characters | `wcslen` / `mbrtowc` / `wcstombs` and others |
| Errors | `errno` / `__errno_location` and the standard error codes |
| Threads | Per-thread control blocks (TCB) and thread-local storage, with `errno` isolated per thread |

Matching C headers live under `include/`; the aggregate header is `boruix.h`.

## Usage

**From Rust**: add it as a dependency (`crate-type` is `rlib` and `staticlib`).

```toml
[dependencies]
libc = { path = "../libc" }
```

```rust
unsafe {
    let p = libc::malloc::malloc(64);
    libc::stdio::snprintf(buf.as_mut_ptr() as *mut i8, buf.len(),
        b"%d %.2f\0".as_ptr() as *const i8, 42, 3.14);
    libc::malloc::free(p);
}
```

**From C**: include the headers under `include/` and link against the static library this crate produces.

## Known limitations

| Limitation | Details |
| --- | --- |
| `errno` is process-wide | There is no kernel thread model yet |
| `lseek` supports absolute positioning only | `SEEK_CUR` / `SEEK_END` return `EINVAL` |
| `waitpid` waits for any child only | Other options return an error |
| File streams are unbuffered by default | Writes go straight to the underlying layer |
| No locale support | Wide characters use a fixed encoding |

## Building and testing

```bash
cargo build --release
cargo test --manifest-path test_harness/Cargo.toml
```

> `src/` contains 19 unit tests and `test_harness/` contains 64.

## Layout

```
libc/
├── include/        # C headers
├── src/            # module implementations
└── test_harness/   # host-side unit tests
```

## Related projects

- [`libsys`](https://github.com/BRX-Boruix/libsys) — the user-space syscall wrapper
- [`csrc`](https://github.com/BRX-Boruix/csrc) — the freestanding C runtime environment

## License

MIT License, copyright Yang Borui. See [LICENSE](LICENSE).
