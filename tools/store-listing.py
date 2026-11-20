"""Microsoft Store listing: checks the listing texts and fills Partner Center's import file.

    python tools/store-listing.py check [<images folder>]
    python tools/store-listing.py fill <exported .csv> <images folder> <import folder>

The listing texts are in packaging/store/listing/<language>.toml (one file per Store
listing language) and packaging/store/listing/common.toml (values shared by every
language). The images come from tools/store-images.py (`screenshots/<language>-<n>.png`,
`video/BetterVolume.mp4`, `video/thumbnail.png`) and the hero image from the same folder
(`hero.png`).

check: every limit Partner Center enforces (lengths, counts, the 21 different words of
the search terms), and, given the images folder, that every listing has its six
screenshots at 1920 x 1080.

fill: Partner Center's "Export listings" file (UTF-8 .csv: Field, ID, Type (its heading
translated into Partner Center's language), default, then one column per listing
language; a "default" value serves every language whose own cell is empty) is copied
into the import folder with every listing value written in, the images copied beside
it; Partner Center's "Import listings → Import folder" then takes the whole folder. An
image or video is named by its path from the folder's parent, the folder's own name
first ("<folder>/<file>"), as Partner Center asks. The Field, ID and Type columns are
never changed. The import folder must not exist yet: it is created only once every value has
found its place. A field with no row in the export, or a listing language with no
column (or several) when the export has language columns, stops the script before
anything is written.
"""

import csv
import os
import shutil
import struct
import sys
import tomllib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LISTING = os.path.join(ROOT, "packaging", "store", "listing")

SCREENSHOTS = 6
LIMITS = {
    "description": 10000,
    "short_description": 1000,
    "feature": 200,
    "features": 20,
    "search_term": 40,
    "search_terms": 7,
    "search_words": 21,
    "caption": 200,
    "trailer_title": 255,
    "copyright": 200,
}
# Shown in full only up to this length in some Store views.
SHORT_DESCRIPTION_SHOWN = 270


def languages():
    """The listing languages, from the file names (`en-US.toml` → `en-US`)."""
    return sorted(
        name[:-5] for name in os.listdir(LISTING) if name.endswith(".toml") and name != "common.toml"
    )


def load(language):
    with open(os.path.join(LISTING, language + ".toml"), "rb") as f:
        return tomllib.load(f)


def common():
    with open(os.path.join(LISTING, "common.toml"), "rb") as f:
        return tomllib.load(f)


def screenshot_prefix(language, folder):
    """The screenshots' language tag for a listing language: the full code when the
    images use it (`pt-BR`, `zh-Hans`), otherwise its first part (`de-DE` → `de`)."""
    names = os.listdir(folder)
    for prefix in (language, language.split("-")[0]):
        if any(n.startswith(prefix + "-") and n[len(prefix) + 1 :].split(".")[0].isdigit() for n in names):
            return prefix
    return None


def png_size(path):
    with open(path, "rb") as f:
        head = f.read(24)
    if len(head) < 24 or head[:8] != b"\x89PNG\r\n\x1a\n":
        return None
    return struct.unpack(">II", head[16:24])


def check(images=None):
    problems = []
    shared = common()
    if len(shared.get("copyright", "")) > LIMITS["copyright"]:
        problems.append("common: copyright too long")
    for language in languages():
        text = load(language)
        where = language

        def limit(value, key, label):
            if len(value) > LIMITS[key]:
                problems.append(f"{where}: {label} has {len(value)} characters (limit {LIMITS[key]})")
            if value != value.strip() and key != "description":
                problems.append(f"{where}: {label} starts or ends with spaces")
            if "http" in value or "www." in value:
                problems.append(f"{where}: {label} holds an address (not allowed in listing texts)")

        description = text["description"].strip()
        limit(description, "description", "description")
        short = text["short_description"]
        limit(short, "short_description", "short description")
        if len(short) > SHORT_DESCRIPTION_SHOWN:
            problems.append(f"{where}: short description has {len(short)} characters (shown in full up to {SHORT_DESCRIPTION_SHOWN})")
        features = text["features"]
        if len(features) > LIMITS["features"]:
            problems.append(f"{where}: {len(features)} features (limit {LIMITS['features']})")
        for i, feature in enumerate(features, 1):
            limit(feature, "feature", f"feature {i}")
        terms = text["search_terms"]
        if len(terms) > LIMITS["search_terms"]:
            problems.append(f"{where}: {len(terms)} search terms (limit {LIMITS['search_terms']})")
        for i, term in enumerate(terms, 1):
            limit(term, "search_term", f"search term {i}")
        words = {w.lower() for term in terms for w in term.split()}
        if len(words) > LIMITS["search_words"]:
            problems.append(f"{where}: {len(words)} different words in the search terms (limit {LIMITS['search_words']})")
        captions = text["captions"]
        if len(captions) != SCREENSHOTS:
            problems.append(f"{where}: {len(captions)} captions for {SCREENSHOTS} screenshots")
        for i, caption in enumerate(captions, 1):
            limit(caption, "caption", f"caption {i}")
        limit(text["trailer_title"], "trailer_title", "trailer title")
        if images:
            folder = os.path.join(images, "screenshots")
            prefix = screenshot_prefix(language, folder)
            if prefix is None:
                problems.append(f"{where}: no screenshots in {folder}")
                continue
            for n in range(1, SCREENSHOTS + 1):
                path = os.path.join(folder, f"{prefix}-{n}.png")
                if not os.path.exists(path):
                    problems.append(f"{where}: missing {path}")
                elif png_size(path) != (1920, 1080):
                    problems.append(f"{where}: {path} is {png_size(path)}, not 1920 x 1080")
        print(f"{language}: description {len(description)}, short description {len(short)}, "
              f"{len(features)} features, {len(terms)} search terms ({len(words)} words)")
    if images:
        for name in ("video/BetterVolume.mp4", "video/thumbnail.png", "hero.png"):
            path = os.path.join(images, name)
            if not os.path.exists(path):
                problems.append(f"missing {name} in {images}")
            elif name.endswith(".png") and png_size(path) != (1920, 1080):
                problems.append(f"{path} is {png_size(path)}, not 1920 x 1080")
    for problem in problems:
        print("PROBLEM:", problem)
    print("LISTING OK" if not problems else f"LISTING HAS {len(problems)} PROBLEM(S)")
    return not problems


def rows_by_field(rows):
    index = {}
    for i, row in enumerate(rows):
        if row and row[0]:
            index[row[0]] = i
    return index


def language_columns(header, wanted):
    """The column of each listing language. When the export already has language columns,
    each language must match exactly one (its code, or a unique column of the same first
    part, e.g. `zh-Hans` against `zh-cn`); otherwise the columns are added."""
    lowered = [h.lower() for h in header]
    existing = lowered[4:]
    columns, problems = {}, []
    for language in wanted:
        code = language.lower()
        if code in existing:
            columns[language] = lowered.index(code)
            continue
        if not any(existing):
            header.append(code)
            columns[language] = len(header) - 1
            continue
        same = [i for i, h in enumerate(lowered) if i >= 4 and h.split("-")[0] == code.split("-")[0]]
        if len(same) == 1:
            columns[language] = same[0]
        else:
            problems.append(f"{language}: {'no' if not same else 'several'} matching column in the export")
    taken = {}
    for language, column in columns.items():
        if column in taken:
            problems.append(f"{taken[column]} and {language} would share the column {header[column]}")
        taken.setdefault(column, language)
    return columns, problems


def fill(exported, images, target):
    if not check(images):
        sys.exit(1)
    target = os.path.abspath(target)
    if os.path.exists(target):
        sys.exit(f"{target} already exists: give a new folder (it is created and filled)")
    with open(exported, encoding="utf-8-sig", newline="") as f:
        rows = list(csv.reader(f))
    # The third heading is translated into Partner Center's language (e.g. "Type (Type)").
    first = rows[0][:4] if rows else []
    if len(first) < 4 or first[:2] != ["Field", "ID"] or not first[2].startswith("Type") or first[3] != "default":
        sys.exit(f"{exported} is not a listing export (first line: {first or 'none'})")
    header = rows[0]
    fields = rows_by_field(rows)
    columns, problems = language_columns(header, languages())
    root = os.path.basename(target)
    copies = []  # (source, name in the import folder)
    missing = set()

    def put(field, column, value):
        if field not in fields:
            missing.add(field)
            return
        row = rows[fields[field]]
        while len(row) <= column:
            row.append("")
        row[column] = value

    def asset(source, name):
        copies.append((source, name))
        # Partner Center wants the folder's own name first, e.g. "my_folder/screenshot1.png".
        return f"{root}/{name}"

    # Every value is placed in memory first; nothing is written before all of them fit.
    shared = common()
    default = 3
    put("CopyrightTrademarkInformation", default, shared["copyright"])
    put("DevStudio", default, shared["developed_by"])
    put("Trailer1", default, asset(os.path.join(images, "video", "BetterVolume.mp4"), "BetterVolume.mp4"))
    put("TrailerThumbnail1", default, asset(os.path.join(images, "video", "thumbnail.png"), "trailer-thumbnail.png"))
    # The 16:9 super hero art: the export's only non-Xbox 1920 x 1080 image row. With it,
    # the trailers play at the top of the listing (TrailerToPlayAtTopOfListing stays empty).
    put("PromoImage1920x1080", default, asset(os.path.join(images, "hero.png"), "hero.png"))
    for language, column in columns.items():
        text = load(language)
        # The reserved name (needed for a language without a package; the same otherwise).
        put("Title", column, shared["title"])
        put("Description", column, text["description"].strip())
        put("ShortDescription", column, text["short_description"])
        for i, feature in enumerate(text["features"], 1):
            put(f"Feature{i}", column, feature)
        for i, term in enumerate(text["search_terms"], 1):
            put(f"SearchTerm{i}", column, term)
        put("TrailerTitle1", column, text["trailer_title"])
        prefix = screenshot_prefix(language, os.path.join(images, "screenshots"))
        for n, caption in enumerate(text["captions"], 1):
            source = os.path.join(images, "screenshots", f"{prefix}-{n}.png")
            put(f"DesktopScreenshot{n}", column, asset(source, f"{language}-{n}.png"))
            put(f"DesktopScreenshotCaption{n}", column, caption)
    if missing:
        problems.append("fields not found in the export: " + ", ".join(sorted(missing)))
    if problems:
        sys.exit("nothing written:\n  " + "\n  ".join(problems))
    width = len(header)
    for row in rows:
        row.extend([""] * (width - len(row)))
    os.makedirs(target)
    for source, name in copies:
        shutil.copyfile(source, os.path.join(target, name))
    out = os.path.join(target, "listings.csv")
    # Same encoding as Partner Center's export (UTF-8 with a byte order mark).
    with open(out, "w", encoding="utf-8-sig", newline="") as f:
        csv.writer(f).writerows(rows)
    print("written", out, "with", len(copies), "files beside it; language columns:",
          ", ".join(f"{l} -> {header[c]}" for l, c in columns.items()))


if __name__ == "__main__":
    if len(sys.argv) >= 2 and sys.argv[1] == "check":
        sys.exit(0 if check(sys.argv[2] if len(sys.argv) > 2 else None) else 1)
    if len(sys.argv) == 5 and sys.argv[1] == "fill":
        fill(sys.argv[2], sys.argv[3], sys.argv[4])
    else:
        sys.exit(__doc__)
