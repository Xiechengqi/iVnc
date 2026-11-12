// Deterministic gesture arbitration. Browser and compositor integration is
// covered by toolbar_smoke.mjs and electron_smoke.mjs.
import assert from 'node:assert/strict';
import { createGestureNavigation } from '../web/ivnc/gesture-navigation.mjs';

let time = 10;
const selected = [];
const source = {}, nextFrame = {};
const nav = createGestureNavigation({
    navigate: direction => selected.push(direction),
    canHandle: origin => origin === source || origin === nextFrame,
    now: () => time,
});
const wheel = (dx, dy = 0, origin = source, extras = {}) => nav.wheel({
    deltaX: dx, deltaY: dy, deltaMode: 0, ...extras,
}, origin);
const later = ms => { time += ms; };

assert.equal(wheel(120), false);
nav.setEnabled(true);
assert.equal(wheel(35), true);
assert.equal(wheel(35), true);
assert.equal(wheel(35), true);
assert.deepEqual(selected, [1]);
assert.equal(wheel(180, 0, nextFrame), true);
assert.deepEqual(selected, [1], 'inertia in the new iframe must not switch twice');
later(300);
assert.equal(wheel(0, 40), false);
assert.equal(wheel(200), false, 'horizontal motion cannot steal an ongoing vertical scroll');
assert.deepEqual(selected, [1]);
later(300);
assert.equal(wheel(4, 4), true);
assert.equal(wheel(4, 50), false, 'a diagonal start returns to ordinary scrolling');
later(300);
nav.setReversed(true);
assert.equal(wheel(120), true);
assert.deepEqual(selected, [1, -1]);
later(300);
assert.equal(wheel(120, 0, source, { ctrlKey: true }), false);
assert.deepEqual(selected, [1, -1]);
assert.equal(wheel(120, 0, source, { deltaMode: 1 }), true);
assert.deepEqual(selected, [1, -1, -1]);
nav.endTouch();
nav.setReversed(false);
assert.equal(nav.touch(-55, 0, source), true);
assert.equal(nav.touch(-55, 0, source), true);
assert.deepEqual(selected, [1, -1, -1, 1]);
nav.endTouch();
assert.equal(nav.touch(-120, 0, source, true), false, 'a pinch or drag must not navigate');
assert.equal(wheel(120, 0, {}, {}), false, 'unregistered views cannot navigate');
nav.setEnabled(false);
assert.equal(wheel(120), false);
console.log('Gesture navigation: 10 behavioral checks passed');
