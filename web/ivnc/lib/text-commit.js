// Receipts confirm protocol dispatch / compatibility-paste delivery, not that
// an application accepted an edit. Never resend an uncertain commit.
export class TextCommitter {
    constructor({ send, sendControl = send, isCurrent, onError }) {
        Object.assign(this, { send, sendControl, isCurrent, onError });
        this.nextId = 0;
        this.pending = new Map();
    }

    transmit(message, control = false) {
        try { return (control ? this.sendControl(message) : this.send(message)) ?? true; } catch (_) { return false; }
    }

    commit(text, state) {
        if (new TextEncoder().encode(text).length > 48 * 1024 || this.pending.size >= 32) {
            this.onError('limit'); return;
        }
        // Old remote runtimes keep their existing native/paste path. Only use
        // the receipt extension when this connected runtime advertises it.
        if (state?.commit !== 1) {
            const sent = this.transmit('t,' + text);
            if (sent?.then) sent.then(ok => { if (ok === false && this.isCurrent(state?.focus)) this.onError('disconnected'); }, () => { if (this.isCurrent(state?.focus)) this.onError('disconnected'); });
            else if (sent === false) this.onError('disconnected');
            return;
        }
        if (!state.focus) { this.onError('no_focus'); return; }
        const id = ++this.nextId;
        const pending = { timer: null, focus: state.focus };
        this.pending.set(id, pending);
        const start = sent => {
            if (this.pending.get(id) !== pending) return;
            if (sent === false) {
                this.pending.delete(id);
                if (this.isCurrent(state.focus)) this.onError('disconnected');
                return;
            }
            pending.timer = setTimeout(() => {
                this.pending.delete(id);
                this.transmit(`ti,${JSON.stringify({ type: 'cancel', id })}`, true);
                if (this.isCurrent(state.focus)) this.onError('timeout');
            }, 3000);
        };
        const sent = this.transmit(`ti,${JSON.stringify({ type: 'commit', id, focus: state.focus, text })}`);
        if (sent?.then) sent.then(start, () => start(false));
        else start(sent);
    }

    result(data) {
        const pending = this.pending.get(data?.id);
        if (!pending) return;
        clearTimeout(pending.timer);
        this.pending.delete(data.id);
        if (!['dispatched', 'applied'].includes(data.status) && this.isCurrent(pending.focus)) this.onError(data.status);
    }

    cancel() {
        for (const [id, { timer }] of this.pending) {
            clearTimeout(timer);
            this.transmit(`ti,${JSON.stringify({ type: 'cancel', id })}`, true);
        }
        this.pending.clear();
    }
}
