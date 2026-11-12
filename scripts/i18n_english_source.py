"""One-time migration (Nov 2026): make English the source language of the UI strings.

Before: `t!("Turkish")` keys, English in crates/rf-app/src/i18n_en.rs, locale files keyed by the
FNV-1a id of the Turkish key. After: `t!("English")` keys, Turkish in locales/tr.json, every
locale file keyed by the id of the English key. This script

  1. builds the English key for every old key (i18n_en.rs, the Linux "Trash" wording, the one
     `tx("Açık", "Light")` pair, the lower crates' texts listed in EXTRA),
  2. rewrites every string literal in crates/**/*.rs that is an old Turkish key,
  3. re-keys locales/<code>.json from old ids to new ids (two old keys with the same English
     merge; the first one with a translation wins) and writes locales/tr.json and _source.json.

Kept for the record; running it again on the migrated tree does nothing useful.
"""
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
LOC = ROOT / "crates" / "rf-app" / "locales"
# (Turkish as written in Rust source, English as it will be written). Lower-crate texts that had
# no English, plus a few rf-app ones.
EXTRA_TEXT = r'''
4 GB'tan büyük sözlük (RAR 7)|dictionaries larger than 4 GB (RAR 7)
7z arşivi okunamadı|Couldn't read the 7z archive
7z oluşturulamadı|Couldn't create the 7z archive
7z sıkıştırma yöntemi ya da başlık biçimi|a 7z compression method or header format
7z'ye yazılamadı: {}|Couldn't write to the 7z archive: {}
Arşiv|Archive
Arşiv açılamadı: {}|Couldn't open the archive: {}
Arşiv bozuk ya da yarım inmiş; yeniden indirmeyi deneyin. (Ayrıntı: {what})|The archive is damaged or only partly downloaded; try downloading it again. (Details: {what})
Arşiv dosyası okunamadı: {x}|Couldn't read the archive file: {x}
Arşiv girdisi okunamadı: {display}|Couldn't read an archive entry: {display}
Arşiv girdisi okunamadı: {name}|Couldn't read an archive entry: {name}
Arşiv güncellenemedi: {}|Couldn't update the archive: {}
Arşiv kaydedilemedi: {}|Couldn't save the archive: {}
Arşiv yazılamadı|Couldn't write the archive
Ayrık HEAD durumunda çekme yapılamaz|Can't pull with a detached HEAD
Ayrık HEAD durumunda gönderilemez|Can't push with a detached HEAD
bilinmeyen başlık türü|an unknown header type
bilinmeyen RAR sıkıştırma yöntemi|an unknown RAR compression method
boyutu kayıtlı olmayan dosyalar|files of unrecorded size
'{branch}' dalının izlediği bir uzak dal yok|Branch '{branch}' doesn't track a remote branch
Bu adda bir öğe zaten var|An item with this name already exists
Bu adla bir öğe zaten var|An item with this name already exists
Bu adla bir öğe zaten var: {}|An item with this name already exists: {}
Bu arşiv biçimi düzenlenemez|This archive format can't be edited
Bu arşiv henüz desteklenmeyen bir özellik kullanıyor: {feature}.|This archive uses a feature that isn't supported yet: {feature}.
Bu arşiv parolalı|This archive is password-protected
Bu arşiv RAR 1.5/2.0'ın eski şifrelemesini kullanıyor; rfiles bunu açmıyor. WinRAR ya da 7-Zip açabilir.|This archive uses the old encryption of RAR 1.5/2.0, which rfiles doesn't open. WinRAR or 7-Zip can.
Bu biçim parolayı desteklemiyor|This format doesn't support passwords
Bu dosya bir arşiv değil ya da başı bozuk.|This file isn't an archive, or its start is damaged.
Bu, RAR 1.3/1.4 biçiminde çok eski bir arşiv (1990'lar); rfiles bu biçimi açmıyor. WinRAR ya da 7-Zip açabilir.|This is a very old archive in the RAR 1.3/1.4 format (1990s), which rfiles doesn't open. WinRAR or 7-Zip can.
çıkış kodu {}|exit code {}
çıktı|output
Çok parçalı arşivin bir parçası eksik. Bütün parçaları (.part1.rar, .part2.rar… ya da .rar, .r00, .r01…) aynı klasöre koyup tekrar deneyin.|A part of the multi-part archive is missing. Put all the parts (.part1.rar, .part2.rar… or .rar, .r00, .r01…) in the same folder and try again.
git status başarısız|git status failed
Değişiklikler stash'e alınamadı|Couldn't stash the changes
Dal değiştirildi ama saklanan değişiklikler geri uygulanamadı (git stash listesinde duruyor)|Switched branch, but the stashed changes couldn't be reapplied (they're still in git stash list)
Uzak depolar okunamadı|Couldn't read the remotes
Depo başlatılamadı|Couldn't initialise the repository
Dal bulunamadı: {branch}|Branch not found: {branch}
Dal değiştirilemedi: {err}|Couldn't switch branch: {err}
Dal değiştirilemedi: yerel değişiklikler çakışıyor. Değişiklikleri saklayın ya da atın.\n{err}|Couldn't switch branch: local changes conflict. Stash or discard them.\n{err}
Desteklenmeyen arşiv türü|Unsupported archive type
Desteklenmeyen arşiv türü: {}|Unsupported archive type: {}
Dosya açılamadı: {}|Couldn't open the file: {}
Dosya açılamadı: {}: {e}|Couldn't open the file: {}: {e}
Dosya okunamadı: {}|Couldn't read the file: {}
Dosya oluşturulamadı: {}|Couldn't create the file: {}
Eklenecek dosya bulunamadı|No files to add were found
Eklenecek öğe yok|Nothing to add
Geçersiz ad|Invalid name
Geçersiz başlangıç noktası: {f}|Invalid starting point: {f}
Geçersiz dal adı: {branch}|Invalid branch name: {branch}
Geçersiz dal adı: {name}|Invalid branch name: {name}
Geçersiz dosya adı|Invalid file name
Geçersiz kaynak yolu|Invalid source path
git status başarısız: {}|git status failed: {}
Güvensiz arşiv girdisi (geçersiz ad): {name}|Unsafe archive entry (invalid name): {name}
Güvensiz arşiv girdisi (mutlak yol): {name}|Unsafe archive entry (absolute path): {name}
Güvensiz arşiv girdisi (yol geçişi): {name}|Unsafe archive entry (path traversal): {name}
Hedef klasör oluşturulamadı|Couldn't create the destination folder
İşlem kullanıcı tarafından durduruldu|Stopped by the user
ISO çok fazla girdi içeriyor|The ISO has too many entries
ISO dosyası açılamadı: {e}|Couldn't open the ISO file: {e}
ISO klasör ağacı çok derin|The ISO folder tree is too deep
ISO klasörü okunamadı: {e}|Couldn't read an ISO folder: {e}
ISO okunamadı (bozuk olabilir): {e}|Couldn't read the ISO (it may be damaged): {e}
ISO okunamadı: {e:?}|Couldn't read the ISO: {e:?}
Kaynak bulunamadı: {}|Source not found: {}
Kaynak dosya açılamadı|Couldn't open a source file
Klasör okunamadı: {}|Couldn't read the folder: {}
Klasör oluşturulamadı|Couldn't create the folder
Klasör oluşturulamadı: {}|Couldn't create the folder: {}
Kopyalanamadı: {name}|Couldn't copy: {name}
\"{name}\" bozuk çıktı (sağlama tutmuyor). Arşiv hasarlı olabilir; yeniden indirmeyi deneyin.|\"{name}\" came out damaged (checksum mismatch). The archive may be damaged; try downloading it again.
Öğe bulunamadı|Item not found
Okunamadı: {}|Couldn't read: {}
Oluşturulamadı: {}|Couldn't create: {}
parçalara bölünmüş ZIP|a ZIP split into parts
Parola yanlış|Wrong password
Parola yanlış.|Wrong password.
RAR arşivi kilitli|The RAR archive is locked
RAR dosyasına yazılamadı (disk dolu olabilir)|Couldn't write the RAR file (the disk may be full)
RAR dosyası oluşturulamadı|Couldn't create the RAR file
Rar.exe başlatılamadı|Couldn't start Rar.exe
RAR için yeterli bellek yok|Not enough memory for RAR
RAR komut satırı hatası|RAR command line error
RAR oluşturulamadı|Couldn't create the RAR archive
RAR oluşturulamadı (CRC hatası)|Couldn't create the RAR archive (CRC error)
RAR oluşturulamadı (ölümcül hata)|Couldn't create the RAR archive (fatal error)
RAR parolası yanlış|Wrong RAR password
şifreleme yöntemi|an encryption method
Silinecek öğe yok|Nothing to delete
Sıkıştırılacak öğe yok|Nothing to compress
Sıkıştırılmış dosya okunamadı|Couldn't read the compressed file
Tar bağlantısı okunamadı|Couldn't read a tar link
Tar girdisi okunamadı|Couldn't read a tar entry
Tar okunamadı|Couldn't read the tar archive
UDF okunamadı: {e}|Couldn't read the UDF file system: {e}
Uzak depo tanımlı değil|No remote is set up
{what} başarısız: {msg}|{what} failed: {msg}
WinRAR bulunamadı|WinRAR not found
yazıcı durdu|the writer stopped
Yazılamadı: {}|Couldn't write: {}
Yazılamadı: {target}|Couldn't write: {target}
Yazma hatası|Write error
ZIP AES şifreleme gücü|a ZIP AES encryption strength
Zip dosyası okunamadı|Couldn't read the zip file
Zip girdi adı çözülemedi|Couldn't decode a zip entry name
Zip girdisi kopyalanamadı|Couldn't copy a zip entry
Zip girdisi okunamadı|Couldn't read a zip entry
ZIP sıkıştırma ya da şifreleme yöntemi (PKWARE güçlü şifreleme olabilir)|a ZIP compression or encryption method (possibly PKWARE strong encryption)
Geçersiz düzenli ifade: {0}|Invalid regular expression: {0}
Klasör bulunamadı: {}|Folder not found: {}
Bu dosya türü döndürülemez|This file type can't be rotated
Çıkarılabilir Disk|Removable Disk
Yerel Disk|Local Disk
Optik Sürücü (CD/DVD)|Optical Drive (CD/DVD)
Bağlı Değil|Not Connected
Disk kullanımı — {}|Disk usage — {}
Arama: {query}|Search: {query}
Çekiliyor|Pulling
Gönderiliyor|Pushing
Eşitleniyor|Syncing
'''
EXTRA = [tuple(l.split('|', 1)) for l in EXTRA_TEXT.strip().split('\n')]
# Literals that match error texts in code, not UI texts of their own.
REWRITE_ONLY = {"Dal değiştirilemedi: yerel değişiklikler": "Couldn't switch branch: local changes", "bulunamadı": "not found"}

# The Linux wording of texts that named a Windows feature: (old key, Turkish, English).
LINUX = [
    ("Geri dönüşüm kutusu", "Çöp kutusu", "Trash"),
    ("Geri Dönüşüm Kutusu", "Çöp Kutusu", "Trash"),
    ("Geri dönüşüm kutusuna taşı", "Çöp kutusuna taşı", "Move to Trash"),
    ("Kutuyu boşalt", "Kutuyu boşalt", "Empty Trash"),
    ("Geri dönüşüm kutusu boş", "Çöp kutusu boş", "The Trash is empty"),
    ("Geri dönüşüm kutusuna taşınıyor:", "Çöp kutusuna taşınıyor:", "Moving to Trash:"),
    ("Geri dönüşüm kutusundan geri yükleniyor:", "Çöp kutusundan geri yükleniyor:", "Restoring from Trash:"),
    ("öğeler geri dönüşüm kutusunda bulunamadı", "öğeler çöp kutusunda bulunamadı", "items were not found in the Trash"),
    ("Öğeler Geri Dönüşüm Kutusu'nda bulunamadı", "Öğeler Çöp Kutusu'nda bulunamadı", "Items were not found in the Trash"),
    ("Geri dönüşüm kutusuna taşınsın mı?", "Çöp kutusuna taşınsın mı?", "Move to the Trash?"),
    ("geri dönüşüm kutusuna taşınacak.", "çöp kutusuna taşınacak.", "will be moved to the Trash."),
    ("Bildirim alanında bir simge kalır; yeni pencere anında açılır", "Sistem tepsisinde bir simge kalır; yeni pencere anında açılır", "An icon stays in the system tray; new windows open instantly"),
]
# New English keys with their Turkish (texts rf-core used to pick by a Turkish/English switch).
MANUAL = [
    ("just now", "az önce"),
    ("{n} min ago", "{n} dakika önce"),
    ("{n} h ago", "{n} saat önce"),
    ("{n} d ago", "{n} gün önce"),
]
# Turkish for an English text that several old keys shared, where the first one isn't the best.
TR_PICK = {"Language": "Dil", "Trash": "Çöp Kutusu", "Home": "Ana Sayfa"}


def fnv(s):
    h = 0x811C9DC5
    for b in s.encode():
        h = ((h ^ b) * 0x01000193) & 0xFFFFFFFF
    return f"{h:08x}"


def unescape(s):
    def rep(m):
        e = m.group(1)
        if e.startswith("u{"):
            return chr(int(e[2:-1], 16))
        return {"n": "\n", "t": "\t", "r": "\r", "0": "\0", "\\": "\\", '"': '"', "'": "'"}[e]
    return re.sub(r"\\(u\{[0-9a-fA-F]+\}|.)", rep, s)


def literals(src):
    """(start, end, raw text) of every plain "…" literal outside comments."""
    i, n = 0, len(src)
    while i < n:
        c = src[i]
        if src.startswith("//", i):
            j = src.find("\n", i)
            i = n if j < 0 else j
        elif src.startswith("/*", i):
            d, i = 1, i + 2
            while i < n and d:
                if src.startswith("/*", i):
                    d, i = d + 1, i + 2
                elif src.startswith("*/", i):
                    d, i = d - 1, i + 2
                else:
                    i += 1
        elif c.isalnum() or c == "_":
            m = re.match(r'(b?r(#*)"|b")|[A-Za-z0-9_]+', src[i:])
            if m.group(1) == 'b"':  # byte string: skip
                j = i + 2
                while src[j] != '"':
                    j += 2 if src[j] == "\\" else 1
                i = j + 1
            elif m.group(1):  # raw string: skip
                h = m.group(2)
                i = src.find('"' + h, i + m.end()) + 1 + len(h)
            else:
                i += m.end()
        elif c == '"':
            j = i + 1
            while src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            yield i, j + 1, src[i + 1 : j]
            i = j + 1
        elif c == "'":
            m = re.match(r"'(\\u\{[0-9a-fA-F]+\}|\\.|[^\\'])'", src[i : i + 12])
            i += m.end() if m else 1
        else:
            i += 1


def main():
    old_source = json.loads((LOC / "_source.json").read_text(encoding="utf-8"))
    # the id a locale file used for a key: listed in _source.json (ids of texts reworded after
    # they were translated, e.g. rustyfiles -> rfiles, kept their old id), else its hash
    old_id = {e["key"]: e["id"] for e in old_source}
    en_rs = (ROOT / "crates/rf-app/src/i18n_en.rs").read_text(encoding="utf-8")
    pairs = re.findall(r'^\s*\("((?:[^"\\]|\\.)*)",\s*"((?:[^"\\]|\\.)*)"\),?$', en_rs, re.M)
    linux = {k: (tr, en) for k, tr, en in LINUX}
    # raw Turkish literal -> raw English literal, and the Turkish text of each English key
    rewrite = {}
    entries = {}  # English (unescaped) -> {"tr": .., "old": [old ids]}

    def add(en_raw, tr_text, old_key):
        en = unescape(en_raw)
        e = entries.setdefault(en, {"tr": None, "old": []})
        e["tr"] = e["tr"] or tr_text
        if old_key is not None:
            e["old"].append(old_id.get(old_key, fnv(old_key)))

    for tr_raw, en_raw in pairs:
        key = unescape(tr_raw)
        if key in linux:
            tr, en = linux[key]
            rewrite[tr_raw] = en
            add(en, tr, key)
        else:
            rewrite[tr_raw] = en_raw
            add(en_raw, key, key)
    for tr_raw, en_raw in EXTRA:
        rewrite.setdefault(tr_raw, en_raw)
        add(en_raw, unescape(tr_raw), unescape(tr_raw))
    for en, tr in MANUAL:
        add(en, tr, None)
    rewrite.update(REWRITE_ONLY)
    for en, tr in TR_PICK.items():
        entries[en]["tr"] = tr
    # tx("Açık", "Light") was keyed "en:Light"
    add("Light", "Açık", "en:Light")

    # 2. rewrite literals (i18n.rs and i18n_en.rs are rewritten by hand); tests and examples keep
    # their Turkish data (collation, search), the ones that check UI texts are fixed by hand
    changed = 0
    for p in sorted((ROOT / "crates").rglob("*.rs")):
        if p.name in ("i18n_en.rs", "i18n.rs") or "examples" in p.parts:
            continue
        src = p.read_text(encoding="utf-8")
        # `#[cfg(test)] mod … {` to its closing brace at column 0
        tests = [(m.start(), src.find("\n}\n", m.end())) for m in re.finditer(r"\n#\[cfg\(test\)\]\n(pub(\(crate\))? )?mod ", src)]
        out, last = [], 0
        for s, e, raw in literals(src):
            if any(a < s < b for a, b in tests):
                continue
            if raw in rewrite and rewrite[raw] != raw:
                out += [src[last:s], '"', rewrite[raw], '"']
                last = e
        if out:
            out.append(src[last:])
            p.write_text("".join(out), encoding="utf-8")
            changed += 1
    print(f"rewrote literals in {changed} files")

    # 3. locale files
    specials = [e for e in old_source if e["key"].startswith("@")]
    notes = {e["en"]: e["note"] for e in old_source if "note" in e and not e["key"].startswith("@")}
    known = {e["id"] for e in old_source}
    source = []
    for en, e in entries.items():
        item = {"id": fnv(en), "en": en}
        if en in notes:
            item["note"] = notes[en]
        if not any(o in known for o in e["old"]):
            item["new"] = True  # no locale had it: translations to come
        source.append((item, e))
    for sp in specials:
        source.append(({"id": sp["id"], "key": sp["key"], "en": sp["en"], **({"note": sp["note"]} if "note" in sp else {})}, {"tr": sp["tr"], "old": [sp["id"]]}))
    ids = [s["id"] for s, _ in source]
    assert len(ids) == len(set(ids)), "id clash"

    for f in sorted(LOC.glob("*.json")):
        if f.name.startswith("_"):
            continue
        old = json.loads(f.read_text(encoding="utf-8"))
        new = {}
        for item, e in source:
            v = next((old[o] for o in e["old"] if o in old), None)
            if v is not None:
                new[item["id"]] = v
        lost = set(old) - {o for _, e in source for o in e["old"]}
        if lost:
            print(f"{f.stem}: {len(lost)} old ids with no key: {sorted(lost)[:5]}")
        f.write_text(json.dumps(new, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    tr = {item["id"]: e["tr"] for item, e in source if e["tr"]}
    (LOC / "tr.json").write_text(json.dumps(tr, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    (LOC / "_source.json").write_text(json.dumps([s for s, _ in source], ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    print(f"{len(source)} keys, {sum('new' in s for s, _ in source)} new")


if __name__ == "__main__":
    main()
