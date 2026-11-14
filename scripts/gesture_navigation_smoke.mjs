// Deterministic trackpad/touch traces. Native view switching is covered by
// toolbar_interaction_test.mjs; compositor integration by toolbar_smoke.mjs.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createGestureNavigation } from '../web/ivnc/gesture-navigation.mjs';

function fixture() {
    let time = 0, active = 'desktop';
    const selected = [];
    const nav = createGestureNavigation({ now: () => time,
        navigate: direction => selected.push({ direction, time }),
        canHandle: origin => origin === active });
    const wheel = (at, dx, dy = 0, origin = active, extras = {}) => {
        time = at;
        return nav.wheel({deltaX:dx,deltaY:dy,deltaMode:0,...extras},origin);
    };
    const touch = (at, dx, dy = 0, blocked = false) => { time = at; return nav.touch(dx,dy,active,blocked); };
    return { nav, wheel, touch, selected, directions: () => selected.map(s => s.direction), activate: value => { active = value; } };
}

test('enabled by default; explicitly disabling preserves normal scrolling', () => {
    const f = fixture();
    assert.equal(f.nav.enabled(),true);
    f.nav.setEnabled(false); assert.equal(f.wheel(0,120),false);
    f.nav.setEnabled(true); f.wheel(1,120);
    assert.deepEqual(f.directions(),[1]);
});

test('horizontal distance is accumulated and a large early peak cannot switch twice', () => {
    const f = fixture();
    for (const [t,dx] of [[0,35],[16,35],[32,35],[48,190],[64,100],[80,50]]) f.wheel(t,dx);
    assert.deepEqual(f.selected,[{direction:1,time:32}]);
});

test('long decaying momentum is consumed across active and inactive iframes', () => {
    const f = fixture(); f.wheel(0,120); f.activate('iframe');
    for (let t=16;t<1600;t+=16) assert.equal(f.wheel(t,120*Math.exp(-t/160),0,t%32 ? 'desktop' : 'iframe'),true);
    assert.deepEqual(f.directions(),[1]);
});

test('a fresh same-direction burst after inertia switches again within 250 ms', () => {
    const f = fixture();
    for (const [t,dx] of [[0,45],[16,40],[32,35],[48,22],[64,15],[80,8],[96,3],[112,1],[160,45],[176,45],[192,45]]) f.wheel(t,dx);
    assert.deepEqual(f.selected,[{direction:1,time:32},{direction:1,time:192}]);
});

test('same-direction swipes rearm after a short pause without a long tail', () => {
    const f = fixture();
    for (const [t,dx] of [[0,55],[16,55],[160,55],[176,55],[320,55],[336,55]]) f.wheel(t,dx);
    assert.deepEqual(f.selected,[{direction:1,time:16},{direction:1,time:176},{direction:1,time:336}]);
});

test('a deliberate reverse swipe is accepted without waiting for wheel silence', () => {
    const f = fixture(); f.wheel(0,120);
    for (const [t,dx] of [[16,60],[32,25],[48,8],[64,3],[112,-40],[128,-40],[144,-40]]) f.wheel(t,dx);
    assert.deepEqual(f.selected,[{direction:1,time:0},{direction:-1,time:144}]);
});

test('minor tail rebounds, sign jitter and isolated spikes cannot re-trigger', () => {
    const f = fixture(); f.wheel(0,120);
    for (const [t,dx] of [[16,50],[32,20],[48,7],[64,3],[100,-3],[116,8],[132,5],[148,2],[164,140],[180,4],[196,3],[212,-8],[228,2]]) f.wheel(t,dx);
    assert.deepEqual(f.directions(),[1]);
});

test('sparsely delivered but still decaying inertia is not a fresh gesture', () => {
    const f = fixture(); f.wheel(0,120);
    for (const [t,dx] of [[100,80],[200,50],[300,30],[400,18],[500,10],[600,5]]) f.wheel(t,dx);
    assert.deepEqual(f.directions(),[1]);
});

test('old iframe momentum informs decay but only the active view can rearm', () => {
    const f = fixture(); f.wheel(0,120); f.activate('iframe');
    for (const [t,dx] of [[16,60],[32,25],[48,8],[64,3],[80,1]]) f.wheel(t,dx,0,'desktop');
    f.wheel(120,55); f.wheel(136,55);
    assert.deepEqual(f.directions(),[1,1]);
    f.wheel(280,-120,0,'desktop'); f.wheel(296,-120,0,'desktop');
    assert.deepEqual(f.directions(),[1,1]);
});

test('vertical scroll cannot be stolen by a later horizontal component', () => {
    const f = fixture();
    assert.equal(f.wheel(0,0,40),false);
    assert.equal(f.wheel(16,200,0),false);
    assert.deepEqual(f.directions(),[]);
    f.wheel(300,120); assert.deepEqual(f.directions(),[1]);
});

test('diagonal beginnings return to vertical scrolling', () => {
    const f = fixture(); assert.equal(f.wheel(0,4,4),true);
    assert.equal(f.wheel(16,4,50),false);
    assert.deepEqual(f.directions(),[]);
});

test('a fresh vertical gesture is not swallowed by old horizontal momentum', () => {
    const f = fixture(); f.wheel(0,120); f.wheel(16,30); f.wheel(32,5);
    assert.equal(f.wheel(144,1,40),false);
    assert.equal(f.wheel(160,100,20),false);
    assert.deepEqual(f.directions(),[1]);
});

test('modifiers and unregistered views do not navigate', () => {
    const f = fixture();
    for (const modifier of ['ctrlKey','metaKey','altKey','shiftKey']) assert.equal(f.wheel(0,120,0,'desktop',{[modifier]:true}),false);
    assert.equal(f.wheel(1,120,0,'other'),false);
    assert.deepEqual(f.directions(),[]);
});

test('line and page wheel deltas normalize to the same direction', () => {
    const f = fixture(); f.wheel(0,5,0,'desktop',{deltaMode:1});
    f.wheel(300,-1,0,'desktop',{deltaMode:2});
    assert.deepEqual(f.directions(),[1,-1]);
});

test('a two-finger touch switches once until explicit release, even with long pauses', () => {
    const f = fixture(); f.touch(0,-55); f.touch(16,-55);
    f.touch(1000,-200); f.touch(2000,200);
    assert.deepEqual(f.directions(),[1]);
    f.nav.endTouch(); f.touch(2001,120);
    assert.deepEqual(f.directions(),[1,-1]);
});

test('pinch/drag blocking never switches an app', () => {
    const f = fixture(); assert.equal(f.touch(0,-120,0,true),false);
    assert.deepEqual(f.directions(),[]);
});

test('unrelated touch cleanup cannot clear wheel momentum suppression', () => {
    const f = fixture(); f.wheel(0,120); f.nav.endTouch(); f.wheel(16,180);
    assert.deepEqual(f.directions(),[1]);
});

test('reset and toggle clear stale input, zero and invalid deltas cannot switch', () => {
    const f = fixture(); f.wheel(0,60); f.nav.reset(); f.wheel(16,60);
    f.wheel(32,NaN); f.wheel(48,Infinity); f.wheel(64,0);
    assert.deepEqual(f.directions(),[]);
    f.nav.setEnabled(false); f.nav.setEnabled(true); f.wheel(80,60);
    assert.deepEqual(f.directions(),[]);
});
