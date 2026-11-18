import test from 'node:test';
import assert from 'node:assert/strict';
import { createScrollInput, normalizeWheel, MAX_SCROLL_PIXELS } from '../web/ivnc/lib/scroll-input.mjs';
import { createGestureNavigation } from '../web/ivnc/gesture-navigation.mjs';

function clock() {
    let now = 0, serial = 0;
    const timers = new Map();
    return { now: () => now,
        schedule: (fn, ms) => { const id = ++serial; timers.set(id, { at: now + ms, fn }); return id; },
        cancel: id => timers.delete(id),
        tick(to) {
            while (true) {
                const next = [...timers].sort((a, b) => a[1].at - b[1].at).find(([, v]) => v.at <= to);
                if (!next) break;
                timers.delete(next[0]); now = next[1].at; next[1].fn();
            }
            now = to;
        },
    };
}
function fixture({ modern = true, speed = 1 } = {}) {
    const time = clock(), messages = [];
    let active = true, accepted = true;
    const input = createScrollInput({ ...time, send: message => { if (!accepted) return false; messages.push(message); return true; }, canSend: () => active, getSpeed: () => speed });
    if (modern) input.setProtocol(1);
    const sum = () => messages.reduce((s, message) => {
        const [kind, x, y] = message.split(','), unit = kind === 'w' ? 12 : 1;
        return [s[0] + Number(x) * unit, s[1] + Number(y) * unit];
    }, [0, 0]);
    return { time, input, messages, sum, active: value => { active = value; }, accepted: value => { accepted = value; } };
}
const close = (a, b) => assert(Math.abs(a - b) < .00001, `${a} != ${b}`);

test('normalizes pixels, lines and actual page dimensions without losing fractions', () => {
    assert.deepEqual(normalizeWheel({ deltaX: .25, deltaY: -1.5 }), { x: .25, y: -1.5 });
    assert.deepEqual(normalizeWheel({ deltaX: 2, deltaY: -3, deltaMode: 1 }), { x: 48, y: -72 });
    assert.deepEqual(normalizeWheel({ deltaX: .5, deltaY: 1, deltaMode: 2 }, 1200, 600), { x: 600, y: 600 });
    for (const event of [{ deltaX: NaN, deltaY: 1 }, { deltaX: 1, deltaY: Infinity }, { deltaX: 1, deltaY: 1, deltaMode: 3 }]) assert.equal(normalizeWheel(event), null);
});

test('60 and 120 Hz traces retain their entire distance rather than ten samples per second', () => {
    for (const hz of [60, 120]) for (const modern of [true, false]) {
        const f = fixture({ modern });
        for (let n = 0; n < hz; n++) { f.time.tick(n * 1000 / hz); f.input.add(0, 10); }
        f.time.tick(1100);
        assert.deepEqual(f.sum(), [0, hz * 10]);
        assert(f.messages.length >= hz * .8, `${hz} Hz: ${f.messages.length} messages`);
    }
});

test('applies speed once and preserves both axes, tiny deltas and direction changes', () => {
    for (const speed of [.25, 1, 4]) {
        const f = fixture({ speed });
        for (let n = 0; n < 120; n++) { f.time.tick(n * 10); f.input.add(.25, -.125); }
        f.time.tick(1300);
        close(f.sum()[0], 30 * speed); close(f.sum()[1], -15 * speed);
        f.input.add(-30, 15); f.time.tick(1400);
        close(f.sum()[0], 0); close(f.sum()[1], 0);
    }
});

test('legacy transport accumulates fractional units and upgrades its remainder exactly once', () => {
    const f = fixture({ modern: false });
    for (let n = 0; n < 47; n++) { f.input.add(0, .25); f.time.tick(n * 10 + 10); }
    assert.deepEqual(f.messages, []);
    f.input.add(0, .25); f.time.tick(500);
    assert.deepEqual(f.messages, ['w,0,1']);
    f.input.add(1.25, -2.75); f.time.tick(520);
    f.input.setProtocol(1); f.time.tick(540);
    const actual=f.messages.at(-1).split(',').slice(1).map(Number);
    assert(Math.abs(actual[0]-1.25)<.047 && Math.abs(actual[1]+2.75)<.047);
});

test('batch cancellation and failed/inactive sends cannot replay into the next target', () => {
    for (const cancel of [f => f.input.reset(), f => f.active(false), f => f.accepted(false)]) {
        const f = fixture(); f.input.add(0, 100); cancel(f); f.time.tick(20);
        assert.deepEqual(f.messages, []);
        f.active(true); f.accepted(true); f.input.add(0, 5); f.time.tick(40);
        assert(Math.abs(f.sum()[1]-5)<.047);
    }
});

test('invalid and pathological deltas are rejected or bounded without capping normal page motion', () => {
    const f = fixture({ speed: 4 });
    f.input.add(Infinity, 10); f.input.add(0, NaN); f.time.tick(20);
    assert.deepEqual(f.messages, []);
    f.input.add(0, 2000); f.time.tick(40); assert(Math.abs(f.sum()[1]-8000)<.047);
    f.input.add(1e12, -1e12); f.time.tick(60);
    assert(f.messages.at(-1).split(',').slice(1).every(v=>Math.abs(Number(v))<=MAX_SCROLL_PIXELS));
});

test('sub-Wayland-quantum motion accumulates and cannot drift on repeated tiny samples', () => {
    const f=fixture();
    for(let n=0;n<1000;n++){f.input.add(0,.001);f.time.tick(n*10+10)}
    assert(Math.abs(f.sum()[1]-1)<.047);
    for(const message of f.messages)assert.equal(Number(message.split(',')[2])/12*256%1,0);
});

function gesture() {
    const time = clock(), replayed = [], switches = [];
    let active = true;
    const nav = createGestureNavigation({ ...time, canHandle: () => active, navigate: d => switches.push(d) });
    const wheel = (x, y) => nav.wheel({ deltaX: x, deltaY: y }, 'view', false, (x, y) => replayed.push([x, y]));
    return { time, nav, wheel, replayed, switches, active: value => { active = value; } };
}

test('a one-pixel vertical sample yields immediately and diagonal reservation is replayed', () => {
    const f = gesture(); assert.equal(f.wheel(0, 1), false);
    f.nav.reset(); assert.equal(f.wheel(4, 4), true); assert.equal(f.wheel(4, 50), false);
    assert.deepEqual(f.replayed, [[4, 4]]); assert.deepEqual(f.switches, []);
});

test('an unfinished ambiguous sample is replayed promptly, while app switches consume it', () => {
    const f = gesture(); assert.equal(f.wheel(2, 2), true); f.time.tick(61);
    assert.deepEqual(f.replayed, [[2, 2]]);
    f.nav.reset(); f.wheel(35, 0); f.wheel(35, 0); f.wheel(35, 0); f.time.tick(150);
    assert.deepEqual(f.switches, [1]); assert.deepEqual(f.replayed, [[2, 2]]);
});

test('reset, origin cancellation and loss of activity discard reserved gestures', () => {
    for (const cancel of [f => f.nav.reset(), f => f.nav.cancelPending('view'), f => f.active(false)]) {
        const f = gesture(); f.wheel(2, 2); cancel(f); f.time.tick(70);
        assert.deepEqual(f.replayed, []);
    }
});

test('lifting fingers replays an ambiguous touch, while cancellation discards it', () => {
    for(const cancelled of [false,true]){
        const f=gesture();
        f.nav.touch(-2,-2,'view',false,(x,y)=>f.replayed.push([x,y]));
        f.nav.endTouch(cancelled);f.time.tick(80);
        assert.deepEqual(f.replayed,cancelled?[]:[[2,2]]);
    }
});
