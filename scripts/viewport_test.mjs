import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

// Exercise the real input methods against measured frame and work-area bounds.
function inputFor({ rect, width, height, fit = 'fill', dpr = 1 }) {
    const source = readFileSync(new URL('../web/ivnc/lib/input2.js', import.meta.url), 'utf8')
        .replace(/^import .*;\s*$/gm, '').replace('export class Input', 'class Input');
    const video = { videoWidth: width, videoHeight: height, style: { objectFit: fit }, getBoundingClientRect: () => rect };
    const element = { getBoundingClientRect: () => rect, parentElement: { clientWidth: 1001, clientHeight: 765 } };
    const ctx = { document: { getElementById: id => id === 'stream' ? video : null }, window: { devicePixelRatio: dpr }, navigator: {}, setTimeout, clearTimeout };
    vm.createContext(ctx); vm.runInContext(source + '\nglobalThis.InputClass = Input;', ctx);
    const sent = [];
    const input = Object.assign(Object.create(ctx.InputClass.prototype), {
        element, useCssScaling: false, buttonMask: 0, _updateCursorPosition() {}, send: message => sent.push(message),
    });
    return { input, sent, element, video };
}

test('mouse and touch use native frame coordinates once on a HiDPI viewport', () => {
    const { input, sent, element } = inputFor({ rect: { left: 30, top: 20, width: 800, height: 400 }, width: 1600, height: 800, dpr: 2 });
    input._mouseButtonMovement({ type: 'mousemove', target: element, clientX: 430, clientY: 220 });
    assert.equal(sent.at(-1), 'm,800,400,0,0');
    input._calculateTouchCoordinates({ clientX: 430, clientY: 220 });
    assert.equal(input.x, 800); assert.equal(input.y, 400);
});

test('manual letterboxing and a scrolled native-size frame map to the displayed content', () => {
    const rect = { left: 30, top: 20, width: 800, height: 600 };
    const { input } = inputFor({ rect, width: 1600, height: 900, fit: 'contain' });
    input._calculateTouchCoordinates({ clientX: 430, clientY: 320 });
    assert.equal(input.x, 800); assert.equal(input.y, 450);
    rect.left = -170; rect.top = -280; rect.width = 1600; rect.height = 900;
    input._calculateTouchCoordinates({ clientX: 10, clientY: 10 });
    assert.equal(input.x, 180); assert.equal(input.y, 290);
    input._calculateTouchCoordinates({ clientX: 3000, clientY: -500 });
    assert.equal(input.x, 1599); assert.equal(input.y, 0);
});

test('automatic resolution uses the work area without applying DPR or accepting a hidden zero-size view', () => {
    const { input, element } = inputFor({ rect: { left: 0, top: 0, width: 1001, height: 765 }, width: 2000, height: 1528, dpr: 2 });
    assert.deepEqual(Array.from(input.getWindowResolution()), [1001, 765]);
    element.parentElement.clientHeight = 0;
    assert.deepEqual(Array.from(input.getWindowResolution()), [1001, 0]);
});
