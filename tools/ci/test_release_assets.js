// Tests for tools/ci/release_assets.js against a fake GitHub (node --test).
'use strict';
const test = require('node:test');
const assert = require('node:assert');
const fs = require('fs');
const os = require('os');
const path = require('path');
const assets = require('./release_assets.js');

function fakeGitHub(releases = []) {
  let nextId = 100;
  const state = { releases, assets: new Map(), calls: [] }; // assets: releaseId -> [{id,name,size,data}]
  const repo = { owner: 'jferments', repo: 'mpfree' };
  const rest = {
    repos: {
      listReleases: 'listReleases',
      listReleaseAssets: 'listReleaseAssets',
      async createRelease(p) {
        state.calls.push(['create', p.tag_name, p.draft]);
        const r = { id: nextId++, tag_name: p.tag_name, draft: p.draft, body: p.body, name: p.name };
        state.releases.push(r);
        return { data: r };
      },
      async updateRelease(p) {
        state.calls.push(['update', p.release_id]);
        const r = state.releases.find(x => x.id === p.release_id);
        Object.assign(r, { body: p.body, target_commitish: p.target_commitish });
        return { data: r };
      },
      async deleteReleaseAsset(p) {
        for (const [rid, list] of state.assets) state.assets.set(rid, list.filter(a => a.id !== p.asset_id));
      },
      async uploadReleaseAsset(p) {
        assert.strictEqual(p.headers['content-length'], p.data.length);
        const list = state.assets.get(p.release_id) || [];
        assert.ok(!list.some(a => a.name === p.name), `duplicate ${p.name}`);
        list.push({ id: nextId++, name: p.name, size: p.data.length, data: Buffer.from(p.data) });
        state.assets.set(p.release_id, list);
      },
    },
  };
  const github = {
    rest,
    async paginate(method, p) {
      if (method === 'listReleases') return state.releases;
      if (method === 'listReleaseAssets') return (state.assets.get(p.release_id) || []).map(({ data, ...a }) => a);
      throw new Error(method);
    },
    async request(route, p) {
      assert.strictEqual(p.headers.accept, 'application/octet-stream');
      for (const list of state.assets.values()) {
        const a = list.find(x => x.id === p.asset_id);
        if (a) return { data: a.data.buffer.slice(a.data.byteOffset, a.data.byteOffset + a.data.length) };
      }
      throw new Error('404');
    },
  };
  const ctx = { github, context: { repo, sha: 'abc' }, core: { info() {} } };
  return { ctx, state };
}

function tmpdir() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'release-assets-'));
}

test('a new draft, files up, down, replaced and removed', async () => {
  const { ctx, state } = fakeGitHub();
  const release = await assets.draft(ctx, 'creator-v1.0.0', 'MPFree USB Creator 1.0.0', 'building');
  assert.deepStrictEqual(state.calls, [['create', 'creator-v1.0.0', true]]);
  const dir = tmpdir();
  const tar = path.join(dir, assets.PROGRAMS);
  fs.writeFileSync(tar, Buffer.from([0, 1, 2, 255]));
  await assets.upload(ctx, release.id, tar);
  const out = path.join(dir, 'down', 'p.tar');
  await assets.download(ctx, release.id, assets.PROGRAMS, out);
  assert.deepStrictEqual(fs.readFileSync(out), Buffer.from([0, 1, 2, 255]));
  fs.writeFileSync(tar, 'second');
  await assets.upload(ctx, release.id, tar); // replaces, never duplicates
  assert.strictEqual(state.assets.get(release.id).length, 1);
  await assert.rejects(assets.download(ctx, release.id, 'missing.exe', out), /has no missing\.exe/);
  const exe = path.join(dir, 'MPFree-USB-Creator-1.0.0-windows-x64.exe');
  fs.writeFileSync(exe, 'exe');
  await assets.upload(ctx, release.id, exe);
  const got = await assets.downloadAll(ctx, release.id, n => n.startsWith('MPFree-USB-Creator-'), path.join(dir, 'rel'));
  assert.deepStrictEqual(got, ['MPFree-USB-Creator-1.0.0-windows-x64.exe']);
  assert.strictEqual(fs.readFileSync(path.join(dir, 'rel', got[0]), 'utf8'), 'exe');
  assert.deepStrictEqual(await assets.remove(ctx, release.id, n => n === assets.PROGRAMS), [assets.PROGRAMS]);
  assert.deepStrictEqual(state.assets.get(release.id).map(a => a.name), ['MPFree-USB-Creator-1.0.0-windows-x64.exe']);
});

test('an existing draft is reused; a published release is refused', async () => {
  const { ctx, state } = fakeGitHub([{ id: 7, tag_name: 'creator-v1.0.0', draft: true },
                                      { id: 8, tag_name: 'creator-v0.9.0', draft: false }]);
  const release = await assets.draft(ctx, 'creator-v1.0.0', 'n', 'b');
  assert.strictEqual(release.id, 7);
  assert.deepStrictEqual(state.calls, [['update', 7]]);
  await assert.rejects(assets.draft(ctx, 'creator-v0.9.0', 'n', 'b'), /already published/);
});

test('a short download is refused', async () => {
  const { ctx, state } = fakeGitHub();
  const release = await assets.draft(ctx, 'creator-v1.0.0', 'n', 'b');
  state.assets.set(release.id, [{ id: 1, name: 'x', size: 10, data: Buffer.from('short') }]);
  await assert.rejects(assets.download(ctx, release.id, 'x', path.join(tmpdir(), 'x')), /got 5 bytes/);
});
