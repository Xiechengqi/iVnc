// Notifications are invalidation hints only: receivers re-read the management
// API. Each instance has a separate origin; order is never sent to remote B.
export function createOrderNotifications(onChange = () => {}) {
    let channel;
    try { channel = new BroadcastChannel('ivnc-app-order'); } catch (_) {}
    const valid = (message) => message?.type === 'ivnc-app-order' && Number.isSafeInteger(message.revision);
    if (channel) channel.onmessage = ({ data }) => { if (valid(data)) onChange(data.revision); };
    const receive = (event) => {
        if (event.origin === window.location.origin && event.source === window.parent && valid(event.data)) {
            onChange(event.data.revision);
        }
    };
    window.addEventListener('message', receive);
    return {
        publish(revision) {
            const message = { type: 'ivnc-app-order', revision };
            channel?.postMessage(message);
            document.querySelectorAll('iframe.web-console-frame').forEach(frame => {
                frame.contentWindow?.postMessage(message, window.location.origin);
            });
        },
        destroy() {
            channel?.close();
            window.removeEventListener('message', receive);
        }
    };
}
