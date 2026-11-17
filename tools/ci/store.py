#!/usr/bin/env python3
"""Keep a CI run's files on the CI VM, never in GitHub's artifact storage.

GitHub counts artifact storage against one allowance for the whole account, by
the hour over the billing month, and once the month's share is used every
upload on the account fails ("Artifact storage quota has been hit", MPFree's
runs since 2026-11-11). So MPFree's workflows upload nothing: a job on the
self-hosted runners copies what its run made (the kernel, the coverage report,
the USB creator's builds) into a folder the four mpfree-ci runners share on
the CI VM, /srv/mpfree-store. jferments/ci-infrastructure's
`mpfree_runner.py install` makes that folder (docs/mpfree-runner.md, "The
store"); the owner copies files out of it with scp.

Layout, for each KIND (kernel, coverage, creator):

    <store>/<KIND>/runs/<run id>.<attempt>/   the files, and run.json (what made them,
                                              each file's sha256 and size)
    <store>/<KIND>/<ref> -> runs/<run>        the newest run for main, pr-<number>,
                                              a tag, or a branch started by hand

Usage (in a job on a self-hosted runner):

    python3 tools/ci/store.py keep KIND PATH... [--keep-days N] [--max-runs N]

copies each PATH (a file, or a folder with what's in it) into the run's folder
under its own name, records it in run.json, points <KIND>/<ref> at the run,
then clears runs of that KIND older than --keep-days (14) or beyond the newest
--max-runs (30). The newest run on main is always kept. Two jobs of one run may
keep files under the same KIND; a name already kept is refused, never
replaced. Nothing is kept if the disk would be left with less than
--min-free-gib (10) free, because After's and PlantGen's builds share it.

It writes only under the store (MPFREE_STORE, default /srv/mpfree-store), and
prints the folder and its size, also to the job's summary.

Exit status: 0 kept; 2 usage or a file missing; 3 no store on this machine;
4 not enough free disk; 5 a name this run already kept.
"""
from __future__ import annotations

import argparse
import datetime as _dt
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sys
import tempfile

DEFAULT_STORE = "/srv/mpfree-store"
KIND_RE = re.compile(r"^[a-z][a-z0-9-]{0,31}$")
RUN_RE = re.compile(r"^[0-9]+\.[0-9]+$")
DIR_MODE = 0o755


class StoreError(Exception):
    def __init__(self, message: str, status: int):
        super().__init__(message)
        self.status = status


def slug(text: str) -> str:
    """A ref as one safe folder name: letters, digits, '.', '_' and '-'."""
    s = re.sub(r"[^A-Za-z0-9._-]+", "-", text).strip("-.") or "unnamed"
    return ("ref-" + s if s == "runs" else s)[:80]


def ref_name(env) -> str:
    """main, pr-<n>, a tag's name, or the branch a hand-started run ran on."""
    ref = env.get("GITHUB_REF", "")
    m = re.fullmatch(r"refs/pull/([0-9]+)/(merge|head)", ref)
    if m:
        return f"pr-{m.group(1)}"
    return slug(env.get("GITHUB_REF_NAME") or ref.rsplit("/", 1)[-1] or "unnamed")


def run_name(env) -> str:
    run_id, attempt = env.get("GITHUB_RUN_ID", ""), env.get("GITHUB_RUN_ATTEMPT", "1")
    if not run_id.isdigit() or not attempt.isdigit():
        raise StoreError("GITHUB_RUN_ID and GITHUB_RUN_ATTEMPT must be set (run this in a workflow job)", 2)
    return f"{run_id}.{attempt}"


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def size_of(path: Path) -> int:
    if path.is_dir() and not path.is_symlink():
        return sum(p.stat().st_size for p in path.rglob("*") if p.is_file() and not p.is_symlink())
    return path.stat().st_size


def files_of(top: Path, name: str):
    """(relative name, path) for a kept file, or for every file under a kept folder."""
    if top.is_dir():
        for p in sorted(top.rglob("*")):
            if p.is_file() and not p.is_symlink():
                yield f"{name}/{p.relative_to(top).as_posix()}", p
    else:
        yield name, top


def open_store(env) -> Path:
    store = Path(env.get("MPFREE_STORE") or DEFAULT_STORE)
    if not store.is_dir() or not os.access(store, os.W_OK | os.X_OK):
        raise StoreError(
            f"{store} is missing or not writable on this runner. It is made on the CI VM by "
            "jferments/ci-infrastructure's mpfree_runner.py install (docs/mpfree-runner.md, 'The store').", 3)
    return store


def kept_at(run: Path) -> float:
    try:
        return float(json.loads((run / "run.json").read_text())["kept_at_epoch"])
    except (OSError, ValueError, KeyError, TypeError):
        return run.stat().st_mtime


def point(kind_dir: Path, ref: str, run: str) -> None:
    """<kind>/<ref> -> runs/<run>, replaced in one step."""
    link = kind_dir / ref
    if link.exists() and not link.is_symlink():
        raise StoreError(f"{link} is not a link; the store is not as this script leaves it", 2)
    tmp = kind_dir / f".{ref}.{os.getpid()}.link"
    tmp.unlink(missing_ok=True)
    os.symlink(f"runs/{run}", tmp)
    os.replace(tmp, link)


def prune(kind_dir: Path, keep_days: float, max_runs: int, now: float) -> list[str]:
    """Clear old runs; keep the one main points to. Returns the runs removed."""
    runs_dir = kind_dir / "runs"
    if not runs_dir.is_dir():
        return []
    main = kind_dir / "main"
    protected = os.path.realpath(main) if main.is_symlink() else None
    runs = sorted((p for p in runs_dir.iterdir() if p.is_dir() and RUN_RE.match(p.name)),
                  key=kept_at, reverse=True)
    removed = []
    for i, run in enumerate(runs):
        if os.path.realpath(run) == protected:
            continue
        if i >= max_runs or now - kept_at(run) > keep_days * 86400:
            shutil.rmtree(run)
            removed.append(run.name)
    for link in kind_dir.iterdir():
        if link.is_symlink() and not link.exists():
            link.unlink()
    for stale in runs_dir.glob(".incoming-*"):
        if now - stale.stat().st_mtime > 86400:
            shutil.rmtree(stale, ignore_errors=True)
    return removed


def keep(kind: str, paths: list[str], env, keep_days: float = 14, max_runs: int = 30,
         min_free_gib: float = 10, now: float | None = None) -> dict:
    if not KIND_RE.match(kind):
        raise StoreError(f"kind {kind!r}: lower-case letters, digits and '-' only", 2)
    if keep_days <= 0 or max_runs < 1:
        raise StoreError("--keep-days must be above 0 and --max-runs at least 1", 2)
    sources = []
    for p in paths:
        path = Path(p)
        if not path.exists() or path.is_symlink():
            raise StoreError(f"{p}: not found (or a link)", 2)
        name = path.resolve().name
        if name in ("run.json", "runs") or name.startswith("."):
            raise StoreError(f"{p}: the name {name!r} is the store's own", 2)
        sources.append((name, path))
    if len({n for n, _ in sources}) != len(sources):
        raise StoreError("two paths with the same name", 2)
    now = _dt.datetime.now(_dt.timezone.utc).timestamp() if now is None else now
    store = open_store(env)
    run, ref = run_name(env), ref_name(env)
    incoming = sum(size_of(p) for _, p in sources)
    free = shutil.disk_usage(store).free
    if free - incoming < min_free_gib * (1 << 30):
        raise StoreError(f"keeping {incoming} bytes would leave less than {min_free_gib} GiB free on "
                         f"{store}'s disk ({free} bytes free); nothing kept", 4)

    kind_dir = store / kind
    runs_dir = kind_dir / "runs"
    with open(store / ".lock", "a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        runs_dir.mkdir(mode=DIR_MODE, parents=True, exist_ok=True)
        run_dir = runs_dir / run
        record_path = run_dir / "run.json"
        record = json.loads(record_path.read_text()) if record_path.is_file() else None
        clash = [n for n, _ in sources if (run_dir / n).exists()]
        if clash:
            raise StoreError(f"this run already kept {', '.join(clash)} under {kind}; nothing replaced", 5)
        # Copy into a folder beside the runs, then move each piece in: a reader
        # never sees half a file under its real name.
        stage = Path(tempfile.mkdtemp(prefix=f".incoming-{run}-", dir=runs_dir))
        try:
            for name, src in sources:
                if src.is_dir():
                    shutil.copytree(src, stage / name, symlinks=False)
                else:
                    shutil.copy2(src, stage / name)
            files = {}
            for name, src in sources:
                for rel, p in files_of(stage / name, name):
                    files[rel] = {"sha256": sha256(p), "bytes": p.stat().st_size}
            run_dir.mkdir(mode=DIR_MODE, exist_ok=True)
            for name, _ in sources:
                os.replace(stage / name, run_dir / name)
        finally:
            shutil.rmtree(stage, ignore_errors=True)
        for d in [run_dir, *(p for p in run_dir.rglob("*") if p.is_dir())]:
            os.chmod(d, DIR_MODE)
        if record is None:
            server = env.get("GITHUB_SERVER_URL", "https://github.com")
            repo = env.get("GITHUB_REPOSITORY", "")
            record = {
                "repository": repo,
                "workflow": env.get("GITHUB_WORKFLOW", ""),
                "run_id": env.get("GITHUB_RUN_ID"),
                "attempt": env.get("GITHUB_RUN_ATTEMPT", "1"),
                "run_url": f"{server}/{repo}/actions/runs/{env.get('GITHUB_RUN_ID')}" if repo else "",
                "event": env.get("GITHUB_EVENT_NAME", ""),
                "ref": env.get("GITHUB_REF", ""),
                "ref_name": ref,
                "sha": env.get("GITHUB_SHA", ""),
                "files": {},
            }
        record["files"].update(files)
        record["kept_at"] = _dt.datetime.fromtimestamp(now, _dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
        record["kept_at_epoch"] = now
        tmp = run_dir / ".run.json.tmp"
        tmp.write_text(json.dumps(record, indent=2, sort_keys=True) + "\n")
        os.chmod(tmp, 0o644)
        os.replace(tmp, record_path)
        point(kind_dir, ref, run)
        removed = prune(kind_dir, keep_days, max_runs, now)
    return {"folder": str(run_dir), "link": str(kind_dir / ref), "files": files, "bytes": incoming,
            "removed_runs": removed}


def report(result: dict, kind: str, env) -> str:
    mb = result["bytes"] / 1e6
    lines = [f"**Kept on the CI VM** ({kind}, {len(result['files'])} files, {mb:.1f} MB): "
             f"`{result['folder']}`, also `{result['link']}` (the newest for this ref). "
             "Copy it with scp from the VM (ci-infrastructure docs/mpfree-runner.md, 'The store'); "
             "nothing went to GitHub's artifact storage."]
    if result["removed_runs"]:
        lines.append(f"Cleared {len(result['removed_runs'])} older {kind} run(s).")
    return "\n\n".join(lines) + "\n"


def main(argv=None, env=None) -> int:
    env = os.environ if env is None else env
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    k = sub.add_parser("keep", help="copy this run's files into the store")
    k.add_argument("kind")
    k.add_argument("paths", nargs="+")
    k.add_argument("--keep-days", type=float, default=14)
    k.add_argument("--max-runs", type=int, default=30)
    k.add_argument("--min-free-gib", type=float, default=10)
    args = parser.parse_args(argv)
    try:
        result = keep(args.kind, args.paths, env, args.keep_days, args.max_runs, args.min_free_gib)
    except StoreError as error:
        print(f"store: {error}", file=sys.stderr)
        return error.status
    text = report(result, args.kind, env)
    print(text, end="")
    summary = env.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
