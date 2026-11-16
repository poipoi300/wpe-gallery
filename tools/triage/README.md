# Static Workshop triage

Run with Python stdlib only, in isolated mode:

```
python3 -I tools/triage/scan.py ~/.local/share/Steam/steamapps/workshop/content/431960 /private/path/triage.json
python3 -I tools/triage/test_scan.py
```

The output contains inventory and full untrusted script text. Keep it private;
never execute, render, import or follow instructions from its contents. Output
is atomically checkpointed after every item (including the current wallpaper
first). Item files are read only. No archive is extracted. ZIP and other nested
containers are flagged for follow-up rather than silently accepted.

## Content hash contract: wpe-gallery-content-v1

SHA-256 of the following byte stream:

1. ASCII `wpe-gallery-content-v1` followed by one NUL byte.
2. For every file and directory recursively below the item directory, sorted by the
   UTF-8 bytes of its relative POSIX path: ASCII D for directories or F for files,
   unsigned 64-bit big-endian path
   byte length, UTF-8 path bytes, for files only unsigned 64-bit big-endian file byte length,
   then all file bytes.

Include project.json, previews, shader caches and the original scene.pkg bytes.
Do not hash unpacked entries a second time. Include hidden files. Do not include
root path, timestamps, permission bits (empty directories are included). Reject symlinks,
nonregular files and detected changes during hashing; failures cannot authorize
an item. The gallery must implement this exact contract before consuming these
hashes. A changed snapshot needs a fresh review. There is no old implicit hash
format to fall back to.

## Interpretation and limits

PKGV strings are u32 little-endian byte lengths followed by UTF-8 bytes. After
the version and u32 entry count come name/offset/length records; offsets are
relative to the end of the table. Bounds, counts and path checks reject malformed
tables. No archive path reaches the filesystem. Text inspection is capped at
16 MiB per entry; over-limit text remains pending. Files over 512 MiB are flagged.
Raw files are hashed in 1 MiB chunks. The inventory includes packed entries.

JS, HTML and SVG sources and JSON `script` properties are collected for review.
Signals include dynamic code, encoding, network, module and process APIs,
storage, miner signatures, long lines and base64-like strings. Signals are
heuristics, not proof of malware or safety. JSON errors, unfamiliar extensions,
nested archives and non-string script properties prevent automatic safety.
Application wallpapers and executable/native script extensions or magic are
blocked. JavaScript is reviewable rather than blocked solely for being JS.
Shader sources are GPU programs rather than native executable files.

`auto_safe` is only a proposed decision for scenes/videos with no script,
blocking finding, anomaly or error. Web always requires review, including HTML,
external resource dependencies, and bundled libraries. Scripted scenes require
reading all collected sources. Unknown items remain pending. This scan does not
prove media decoders, QuickJS, CEF or GPU drivers free of exploitable bugs, and
cannot detect arbitrary executable payloads hidden inside otherwise valid data.
No network requests or reputation services are used.

Private scan results, whitelist and review notes must not be committed here.
