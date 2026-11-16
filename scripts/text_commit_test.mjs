import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
// Import the production module without changing the frontend package's mode.
const { TextCommitter } = await import('data:text/javascript;base64,' + Buffer.from(readFileSync(new URL('../web/ivnc/lib/text-commit.js', import.meta.url))).toString('base64'));
const fixture = () => {
    const sent = [], errors = [];
    const state = { commit: 1, focus: 'editor' };
    let current = true, open = true;
    const committer = new TextCommitter({ send: message => { if (!open) return false; sent.push(message); return true; }, isCurrent: () => current, onError: reason => errors.push(reason) });
    return { sent, errors, state, committer, close: () => { open = false; }, deactivate: () => { current = false; } };
};

test('receipt-capable and old runtimes use distinct, lossless wire formats', () => {
    const f = fixture();
    f.committer.commit('中文,🙂', f.state);
    assert.deepEqual(JSON.parse(f.sent[0].slice(3)), { type: 'commit', id: 1, focus: 'editor', text: '中文,🙂' });
    f.committer.result({ id: 1, status: 'dispatched' });
    assert.equal(f.committer.pending.size, 0);
    f.committer.commit('旧版,中文', { protocol: 1 });
    assert.equal(f.sent[1], 't,旧版,中文');
    assert.deepEqual(f.errors, []);
});

test('actual failures are reported, stale/duplicate results are ignored', () => {
    const f = fixture();
    f.committer.commit('中文', f.state);
    f.committer.result({ id: 9, status: 'no_focus' });
    assert.equal(f.committer.pending.size, 1);
    f.committer.result({ id: 1, status: 'focus_changed' });
    f.committer.result({ id: 1, status: 'delivery_failed' });
    assert.deepEqual(f.errors, ['focus_changed']);
    f.close(); f.committer.commit('中文', f.state);
    assert.deepEqual(f.errors, ['focus_changed', 'disconnected']);
    assert.equal(f.committer.pending.size, 0);
});

test('missing target and byte/queue limits never send text', () => {
    const f = fixture();
    f.committer.commit('中文', { commit: 1, focus: null });
    f.committer.commit('中'.repeat(16385), f.state);
    assert.deepEqual(f.errors, ['no_focus', 'limit']);
    assert.equal(f.sent.length, 0);
    for (let n = 0; n < 33; n++) f.committer.commit('中', f.state);
    assert.equal(f.sent.length, 32);
    assert.equal(f.errors.at(-1), 'limit');
    f.committer.cancel();
});

test('uncertain commits time out once and are never retried', t => {
    t.mock.timers.enable({ apis: ['setTimeout'] });
    const f = fixture();
    f.committer.commit('中文', f.state);
    t.mock.timers.tick(3000);
    assert.deepEqual(f.errors, ['timeout']);
    assert.equal(f.sent.length, 2);
    assert.deepEqual(JSON.parse(f.sent[1].slice(3)), { type: 'cancel', id: 1 });
    t.mock.timers.tick(6000);
    assert.equal(f.sent.length, 2);
    assert.equal(f.committer.pending.size, 0);
});

test('switching views cancels pending commits and suppresses late errors', t => {
    t.mock.timers.enable({ apis: ['setTimeout'] });
    const f = fixture();
    f.committer.commit('中文', f.state);
    f.committer.cancel();
    f.committer.result({ id: 1, status: 'no_focus' });
    t.mock.timers.tick(3000);
    assert.deepEqual(f.errors, []);
    assert.equal(f.committer.pending.size, 0);
    f.committer.commit('下一次', f.state);
    assert.equal(JSON.parse(f.sent.at(-1).slice(3)).id, 2);
    f.deactivate(); f.committer.result({ id: 2, status: 'no_focus' });
    assert.deepEqual(f.errors, []);
});

test('waiting behind a clipboard transfer does not consume the receipt timeout', async t => {
    t.mock.timers.enable({ apis: ['setTimeout'] });
    let sent;
    const errors = [], controls = [];
    const committer = new TextCommitter({ send: () => new Promise(resolve => { sent = resolve; }), sendControl: message => controls.push(message), isCurrent: () => true, onError: reason => errors.push(reason) });
    committer.commit('中文', { commit: 1, focus: 'editor' });
    t.mock.timers.tick(10000);
    assert.deepEqual(errors, []);
    sent(true); await Promise.resolve(); t.mock.timers.tick(3000);
    assert.deepEqual(errors, ['timeout']);
    assert.equal(controls.length, 1);
    committer.commit('第二次', { commit: 1, focus: 'editor' });
    committer.cancel();sent(true);await Promise.resolve();t.mock.timers.tick(3000);
    assert.deepEqual(errors, ['timeout']);
    assert.equal(committer.pending.size, 0);
});
