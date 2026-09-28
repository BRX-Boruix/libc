# libc

The BORUIX C standard library: implemented in Rust, exporting the standard C interface on top of [`libsys`](https://github.com/BRX-Boruix/libsys).

[简体中文](README.md)

C headers live under `include/`, with `boruix.h` as the aggregate header.

## Implemented

- Memory — `malloc`, `free`, `realloc`, `calloc`, `posix_memalign`, `aligned_alloc`
- Strings and memory blocks — `memcpy`, `strlen`, `strcmp`, `strstr`, `strtok_r` and more
- Formatted output — `printf`, `snprintf`, `fprintf`, `vprintf` and a full format engine (`%f`, `%e`, `%g`)
- Formatted input — the `fscanf` family
- File streams — `fopen`, `fread`, `fwrite`, `fgets`, `fputs`, `fflush` and more
- Conversion and sorting — `strtol`, `strtod`, `atoi`, `qsort`, `bsearch`
- Character classification — `isalpha`, `tolower`, `toupper` and more
- Syscall wrappers — `open`, `read`, `write`, `lseek`, `unlink`, `chdir`, `getcwd`, `isatty`
- Process and time — `exit`, `getpid`, `kill`, `waitpid`, `time`, `nanosleep`
- Wide characters — `wcslen`, `mbrtowc`, `wcstombs` and more
- Threads — per-thread control blocks and thread-local storage, with `errno` isolated per thread

## Usage

**Rust programs**: add it as a dependency (crate types rlib and staticlib):

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

**C programs**: include the headers under `include/` and link the produced static library.

## Known limitations

- `errno` isolation needs the thread to be spawned through this library's bootstrap; unbootstrapped threads share one fallback slot
- `lseek` supports absolute positioning only; `SEEK_CUR` and `SEEK_END` return `EINVAL`
- `waitpid` waits for any child only; other options return errors
- File streams are unbuffered by default and go straight to the layer below
- No locale; wide characters use a fixed encoding

## Building and testing

```bash
cargo build --release
cargo test
cargo test --manifest-path test_harness/Cargo.toml
```

The library carries 19 unit tests; `test_harness/` is a host-side test program with 64 test functions
covering the format engine, conversions, sorting and searching.

## Layout

```
libc/
├── include/        # C headers
├── src/            # module implementations
└── test_harness/   # host-side tests
```

## Related projects

- [`libsys`](https://github.com/BRX-Boruix/libsys) — user-space syscall wrappers
- [`csrc`](https://github.com/BRX-Boruix/csrc) — the freestanding C runtime

## License

MIT License, copyright Yang Borui. See [LICENSE](LICENSE).
