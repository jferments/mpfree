// The USB creator release's jobs hand files to each other through the draft
// release itself, never through GitHub's artifact storage (one allowance for the
// whole account, billed by the hour, full since 2026-11-11). The Windows, macOS
// and Linux creators are built on GitHub's own machines, which can't reach the
// CI VM's store (tools/ci/store.py), so the programs go up to the draft as
// build-stick-programs.tar, the creators come back into it, and the last job
// takes the .tar out again. Release files don't count against that storage, and
// the draft is where the creators are published from anyway.
//
// From actions/github-script, after actions/checkout:
//   const assets = require('./tools/ci/release_assets.js');
//   await assets.upload({github, context, core}, releaseId, 'out/file.exe');
const fs = require('fs');
const path = require('path');

const PROGRAMS = 'build-stick-programs.tar';

async function list({ github, context }, releaseId) {
  return github.paginate(github.rest.repos.listReleaseAssets,
    { ...context.repo, release_id: releaseId, per_page: 100 });
}

// The draft for this tag: an existing draft is reused, a published one is refused.
async function draft({ github, context, core }, tag, name, body) {
  const releases = await github.paginate(github.rest.repos.listReleases, { ...context.repo, per_page: 100 });
  const existing = releases.find(r => r.tag_name === tag);
  if (existing && !existing.draft) {
    throw new Error(`${tag} is already published; raise the version in rust/mpfree-creator/Cargo.toml`);
  }
  const release = existing
    ? (await github.rest.repos.updateRelease({ ...context.repo, release_id: existing.id, body,
        target_commitish: context.sha })).data
    : (await github.rest.repos.createRelease({ ...context.repo, tag_name: tag, target_commitish: context.sha,
        name, body, draft: true, prerelease: true })).data;
  core.info(`draft release ${tag}: id ${release.id}`);
  return release;
}

async function remove(ctx, releaseId, test) {
  const { github, context, core } = ctx;
  const gone = [];
  for (const a of await list(ctx, releaseId)) {
    if (test(a.name)) {
      await github.rest.repos.deleteReleaseAsset({ ...context.repo, asset_id: a.id });
      gone.push(a.name);
    }
  }
  if (gone.length) core.info(`removed ${gone.join(', ')}`);
  return gone;
}

// Upload a file as an asset, replacing one of the same name.
async function upload(ctx, releaseId, file, name = path.basename(file)) {
  const { github, context, core } = ctx;
  const data = fs.readFileSync(file);
  await remove(ctx, releaseId, n => n === name);
  await github.rest.repos.uploadReleaseAsset({ ...context.repo, release_id: releaseId, name, data,
    headers: { 'content-type': 'application/octet-stream', 'content-length': data.length } });
  core.info(`uploaded ${name} (${data.length} bytes)`);
}

async function fetchAsset(ctx, asset, dest) {
  const { github, context, core } = ctx;
  const res = await github.request('GET /repos/{owner}/{repo}/releases/assets/{asset_id}',
    { ...context.repo, asset_id: asset.id, headers: { accept: 'application/octet-stream' } });
  const data = Buffer.from(res.data);
  if (data.length !== asset.size) {
    throw new Error(`${asset.name}: got ${data.length} bytes, the release says ${asset.size}`);
  }
  fs.mkdirSync(path.dirname(dest), { recursive: true });
  fs.writeFileSync(dest, data);
  core.info(`downloaded ${asset.name} (${data.length} bytes)`);
}

// Download one named asset to dest.
async function download(ctx, releaseId, name, dest) {
  const asset = (await list(ctx, releaseId)).find(a => a.name === name);
  if (!asset) throw new Error(`the draft release (id ${releaseId}) has no ${name}`);
  await fetchAsset(ctx, asset, dest);
}

// Download every asset whose name passes test into dir; returns their names.
async function downloadAll(ctx, releaseId, test, dir) {
  const names = [];
  for (const a of await list(ctx, releaseId)) {
    if (!test(a.name)) continue;
    if (a.name !== path.basename(a.name) || a.name.startsWith('.')) throw new Error(`asset name ${a.name}`);
    await fetchAsset(ctx, a, path.join(dir, a.name));
    names.push(a.name);
  }
  return names;
}

module.exports = { PROGRAMS, draft, remove, upload, download, downloadAll };
