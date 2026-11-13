// Native SDP often advertises H.264 level 3.1 even when the receiver supports
// larger desktop frames. Raise only a receiving codec's level after a WebRTC
// MediaCapabilities probe confirms it; never infer this from a browser name.
export async function prepareVideoOffer(offer, mediaCapabilities = globalThis.navigator?.mediaCapabilities) {
    const original = { type: offer.type, sdp: offer.sdp };
    if (!mediaCapabilities?.decodingInfo) return original;
    const sections = offer.sdp.split(/(?=^m=)/m);
    const video = sections.find(s => s.startsWith('m=video ') && !/^m=video 0 /m.test(s));
    if (!video || /^a=(sendonly|inactive)\r?$/m.test(video)) return original;
    const lines = video.split('\n');
    const pts = new Set(lines[0].trim().split(/\s+/).slice(3));
    const candidates = lines.filter(line => {
        const fmtp = line.match(/^a=fmtp:(\d+) (.*)/);
        if (!fmtp || !pts.has(fmtp[1]) || !lines.some(l => l.trim().toLowerCase() === `a=rtpmap:${fmtp[1]} h264/90000`)) return false;
        const params = new Map(fmtp[2].trim().split(';').map(p => p.trim().split('=')));
        return params.get('packetization-mode') === '1' && /^42[ce]0[0-9a-f]{2}$/i.test(params.get('profile-level-id') || '');
    });
    if (!candidates.length) return original;
    let timer;
    const probe = async () => {
        for (const [level, width, height] of [[52, 4096, 2160], [42, 1920, 1080]]) {
            try {
                const result = await mediaCapabilities.decodingInfo({ type: 'webrtc', video: {
                    contentType: `video/H264;profile-level-id=42e0${level.toString(16)};packetization-mode=1`,
                    width, height, bitrate: 8000000, framerate: 60,
                } });
                if (result.supported) return level;
            } catch (_) { /* An engine may reject the higher level only. */ }
        }
        return null;
    };
    const level = await Promise.race([probe(), new Promise(resolve => { timer = setTimeout(() => resolve(null), 1500); })]);
    clearTimeout(timer);
    if (!level) return original;
    const updated = lines.map(line => candidates.includes(line) ? line.replace(/(profile-level-id=42[ce]0)([0-9a-f]{2})/i,
        (match, prefix, previous) => parseInt(previous, 16) < level ? prefix + level.toString(16) : match) : line).join('\n');
    return { type: offer.type, sdp: sections.map(section => section === video ? updated : section).join('') };
}
