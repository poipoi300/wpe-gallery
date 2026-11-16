#!/usr/bin/env python3
"""Read-only Workshop triage. Never imports, executes, renders or extracts item data."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import struct
import tempfile

TEXT_LIMIT = 16 * 1024 * 1024
BLOCK = set('.exe .dll .com .bat .cmd .ps1 .vbs .vbe .jar .scr .lnk .msi .py .pyc .sh .bash .zsh .fish .pl .rb .hta .wsf .wsh .cpl .sys .so .dylib .app .docm .xlsm .pptm .dotm .xlam'.split())
KNOWN = set('.json .pkg .jpg .jpeg .png .gif .webp .bmp .ico .mp4 .webm .avi .mov .mkv .wmv .m4v .mp3 .ogg .wav .flac .tex .mdl .vtx .vvd .vmt .vtf .obj .mtl .fbx .ttf .otf .woff .woff2 .txt .md .html .htm .css .js .mjs .glsl .vert .frag .hlsl .fx .material .svg .ini .xml .csv .srt .vtt .effect .mesh .scene'.split())
PATTERNS = {
 'network': r'\b(?:fetch|XMLHttpRequest|WebSocket|EventSource|sendBeacon)\b|https?://|wss?://',
 'module_io': r'\b(?:require|import)\s*\(|\b(?:child_process|execSync|spawnSync|readFile|writeFile|Deno|ActiveXObject)\b|\b(?:fs|process)\s*\.',
 'dynamic_code': r'\beval\s*\(|\b(?:new\s+)?(?-i:Function)\s*\(|\batob\s*\(|\bfromCharCode\s*\(',
 'storage': r'\b(?:localStorage|sessionStorage|document\.cookie)\b',
 'miner': r'coinhive|cryptonight|stratum\+|webminer|deepminer|\bWebAssembly\b',
 'packed': r'eval\s*\(function\s*\(p,a,c,k,e|(?:\\x[0-9a-fA-F]{2}){20}|[A-Za-z0-9+/]{1000,}={0,2}',
}

def sniff(b):
    if b.startswith(b'MZ'): return 'PE/DOS'
    if b.startswith(b'\x7fELF'): return 'ELF'
    if b[:4] in (bytes.fromhex(x) for x in ('feedface','cefaedfe','feedfacf','cffaedfe','cafebabe','bebafeca')): return 'Mach-O/Java'
    if b.startswith(b'#!'): return 'script-shebang'
    if b.startswith((b'PK\x03\x04', b'7z\xbc\xaf\x27\x1c', b'Rar!', b'\x1f\x8b', b'BZh', b'\xfd7zXZ', b'\xd0\xcf\x11\xe0')): return 'archive/container'
    return None

def package_table(f, size):
    def u32():
        b = f.read(4)
        if len(b) != 4: raise ValueError('truncated table')
        return struct.unpack('<I', b)[0]
    def string():
        n = u32()
        if n > 65536: raise ValueError('oversized table string')
        b = f.read(n)
        if len(b) != n: raise ValueError('truncated string')
        return b.decode('utf-8', 'strict')
    version = string()
    if not re.fullmatch(r'PKGV00\d\d', version): raise ValueError('unknown PKGV version')
    count = u32()
    if count > 100000: raise ValueError('oversized table')
    rows = [(string(), u32(), u32()) for _ in range(count)]
    base = f.tell()
    seen = set()
    for name, offset, length in rows:
        if name in seen or name.startswith(('/', '\\')) or '..' in name.replace('\\','/').split('/'):
            raise ValueError('duplicate/unsafe package path')
        seen.add(name)
        if offset + length > size - base: raise ValueError('package entry outside data')
    return base, rows

def atomic_json(path, value):
    path = Path(path); path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=path.parent, prefix=path.name+'.')
    try:
        with os.fdopen(fd, 'w') as f:
            json.dump(value, f, ensure_ascii=True, indent=2); f.write('\n'); f.flush(); os.fsync(f.fileno())
        os.replace(tmp, path)
    finally:
        if os.path.exists(tmp): os.unlink(tmp)

def scan(root):
    out = {'id': root.name, 'type': 'unknown', 'inventory': [], 'scripts': [], 'flags': [], 'blocked': [], 'errors': []}
    h = hashlib.sha256(); h.update(b'wpe-gallery-content-v1\0')
    def inspect(name, size, head, data=None):
        ext = Path(name).suffix.lower(); magic = sniff(head)
        out['inventory'].append({'path': name, 'size': size, 'ext': ext, 'magic': magic})
        if ext in BLOCK or magic in ('PE/DOS','ELF','Mach-O/Java','script-shebang'):
            out['blocked'].append(name+': '+(magic or ext))
        if ext not in KNOWN: out['flags'].append('unexpected extension: '+name)
        if magic == 'archive/container': out['flags'].append('nested container (not expanded): '+name)
        if size > 512*1024*1024: out['flags'].append('large file: '+name)
        if ext in ('.html','.htm','.js','.mjs','.json','.svg'):
            if data is None: out['errors'].append('text exceeds limit: '+name); return
            text = data.decode('utf-8-sig', 'replace')
            if ext == '.json':
                try: obj = json.loads(text)
                except (ValueError, RecursionError):
                    out['errors'].append('invalid JSON: '+name); return
                def walk(v, loc):
                    if isinstance(v, dict):
                        for k,x in v.items():
                            if k.lower() == 'script':
                                if isinstance(x, str): add_script(loc+'.script', x)
                                else: out['flags'].append('non-string script property: '+loc)
                            walk(x, loc+'.'+k)
                    elif isinstance(v, list):
                        for i,x in enumerate(v): walk(x, loc+f'[{i}]')
                try: walk(obj, name)
                except RecursionError: out['errors'].append('deep JSON: '+name)
            else: add_script(name, text)
    def add_script(name, text):
        signals = [k for k,p in PATTERNS.items() if re.search(p,text,re.I)]
        if max(map(len,text.splitlines()), default=0)>2000: signals.append('long-line')
        out['scripts'].append({'path': name, 'text': text, 'signals': signals, 'size': len(text)})
    try:
        if root.is_symlink(): raise ValueError('symlink item root')
        paths = []
        for directory, dirs, files in os.walk(root, followlinks=False):
            for n in dirs+files:
                p = Path(directory)/n
                if p.is_symlink(): raise ValueError('symlink: '+str(p.relative_to(root)))
            paths.extend(Path(directory)/n for n in dirs+files)
        paths.sort(key=lambda p: p.relative_to(root).as_posix().encode('utf-8'))
        for p in paths:
            rel = p.relative_to(root).as_posix(); before = p.stat()
            name = rel.encode('utf-8')
            is_dir = stat.S_ISDIR(before.st_mode)
            h.update(b'D' if is_dir else b'F'); h.update(struct.pack('>Q',len(name))); h.update(name)
            if is_dir: continue
            if not stat.S_ISREG(before.st_mode): raise ValueError('nonregular: '+rel)
            h.update(struct.pack('>Q',before.st_size))
            with p.open('rb') as f:
                head = f.read(512); f.seek(0)
                while b := f.read(1024*1024): h.update(b)
                f.seek(0); data = f.read() if before.st_size <= TEXT_LIMIT and p.suffix.lower() in ('.json','.js','.mjs','.html','.htm','.svg') else None
                inspect(rel,before.st_size,head,data)
                if p.suffix.lower()=='.pkg':
                    f.seek(0)
                    try:
                        base, rows = package_table(f,before.st_size)
                        for n,o,s in rows:
                            f.seek(base+o); head=f.read(min(512,s)); f.seek(base+o)
                            data=f.read(s) if s<=TEXT_LIMIT and Path(n).suffix.lower() in ('.json','.js','.mjs','.html','.htm','.svg') else None
                            inspect(rel+'::'+n,s,head,data)
                    except (ValueError, UnicodeError, struct.error) as e: out['errors'].append(rel+': '+str(e))
            after=p.stat()
            if (before.st_size,before.st_mtime_ns,before.st_ino)!=(after.st_size,after.st_mtime_ns,after.st_ino): raise ValueError('changed during scan: '+rel)
        after_paths = sorted((Path(d)/n for d,ds,fs in os.walk(root, followlinks=False) for n in ds+fs), key=lambda p:p.relative_to(root).as_posix().encode('utf-8'))
        if after_paths != paths: raise ValueError('directory changed during scan')
        project=json.loads((root/'project.json').read_text())
        out['type']=str(project.get('type','unknown')).lower()
        out['entrypoint']=project.get('file')
        entry = out['entrypoint']
        if not isinstance(entry, str) or not any(row['path'] == entry or row['path'] == 'scene.pkg::'+entry for row in out['inventory']):
            out['flags'].append('missing/unresolved project entrypoint')
        if out['type']=='application': out['blocked'].append('application wallpaper')
        out['content_hash']=h.hexdigest()
    except (OSError,ValueError,RecursionError) as e: out['errors'].append(str(e))
    out['flags']=sorted(set(out['flags']))
    out['auto_safe']=out['type'] in ('scene','video') and not any(out[k] for k in ('scripts','flags','blocked','errors'))
    return out

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('root',type=Path); ap.add_argument('output',type=Path)
    args=ap.parse_args(); results={}
    for p in sorted(args.root.iterdir(),key=lambda p:(p.name!='3633967177',p.name)):
        if p.is_dir() and p.name.isdecimal():
            results[p.name]=scan(p); atomic_json(args.output,results)
            print(p.name,results[p.name]['type'],len(results[p.name]['scripts']),len(results[p.name]['blocked']),flush=True)
if __name__=='__main__': main()
