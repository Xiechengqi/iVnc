// Notifications are invalidation hints only: receivers re-read the management
// API. Each instance has a separate origin; order and aliases stay local to A.
export function createOrderNotifications(onChange = () => {}, onNameChange = () => {}) {
    let channel;
    try { channel = new BroadcastChannel('ivnc-app-order'); } catch (_) {}
    const receiveHint = (message) => {
        if (!Number.isSafeInteger(message?.revision) || message.revision < 0) return;
        if (message.type === 'ivnc-app-order') onChange(message.revision);
        if (message.type === 'ivnc-app-name' && typeof message.id === 'string') onNameChange(message.id, message.revision);
    };
    if (channel) channel.onmessage = ({ data }) => receiveHint(data);
    const receive = (event) => {
        if (event.origin === window.location.origin && event.source === window.parent) {
            receiveHint(event.data);
        }
    };
    window.addEventListener('message', receive);
    function publish(message) {
        channel?.postMessage(message);
        document.querySelectorAll('iframe.web-console-frame').forEach(frame => {
            frame.contentWindow?.postMessage(message, window.location.origin);
        });
    }
    return {
        publish(revision) { publish({ type: 'ivnc-app-order', revision }); },
        publishName(id, revision) { publish({ type: 'ivnc-app-name', id, revision }); },
        destroy() {
            channel?.close();
            window.removeEventListener('message', receive);
        }
    };
}
