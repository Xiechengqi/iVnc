// One work area for the desktop, input surface and embedded application views.
export function createViewport({ root, container, video, overlay, toolbar, isPinned, embedded, onChange }) {
    let disposed = false, frame = 0, manual = null, previousSize = '', pixelRatioQuery;
    const set = (element, property, value) => {
        if (element.style[property] !== value) element.style[property] = value;
    };

    function layout() {
        if (disposed || !container.isConnected) return;
        const fullscreen = document.fullscreenElement;
        const reservesToolbar = !embedded && isPinned() && toolbar.isConnected
            && (!fullscreen || fullscreen.contains(toolbar));
        const reserved = reservesToolbar ? toolbar.getBoundingClientRect().height : 0;
        root.style.setProperty('--ivnc-toolbar-height', `${reserved}px`);
        container.classList.toggle('manual-exact', !!manual && !manual.fit);
        const width = container.clientWidth, height = container.clientHeight;
        let left = 0, top = 0, videoWidth = width, videoHeight = height;
        if (manual) {
            const scale = manual.fit ? Math.min(width / manual.width, height / manual.height) : 1;
            videoWidth = manual.width * scale;
            videoHeight = manual.height * scale;
            if (manual.fit) { left = (width - videoWidth) / 2; top = (height - videoHeight) / 2; }
        }
        for (const element of [video, overlay]) {
            set(element, 'left', `${left}px`); set(element, 'top', `${top}px`);
            set(element, 'width', `${videoWidth}px`); set(element, 'height', `${videoHeight}px`);
        }
        set(video, 'objectFit', manual ? 'contain' : 'fill');
        const size = `${width}x${height}@${window.devicePixelRatio}`;
        const changed = size !== previousSize;
        previousSize = size;
        onChange({ width, height, changed });
    }
    function schedule() {
        if (!disposed && !frame) frame = requestAnimationFrame(() => { frame = 0; layout(); });
    }
    function watchPixelRatio() {
        pixelRatioQuery?.removeEventListener('change', pixelRatioChanged);
        pixelRatioQuery = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
        pixelRatioQuery.addEventListener('change', pixelRatioChanged);
    }
    function pixelRatioChanged() { watchPixelRatio(); schedule(); }
    const observer = new ResizeObserver(schedule);
    observer.observe(container); observer.observe(toolbar);
    window.addEventListener('resize', schedule);
    document.addEventListener('fullscreenchange', schedule);
    container.addEventListener('scroll', schedule, { passive: true });
    watchPixelRatio();
    layout();
    return {
        layout,
        automatic() { manual = null; container.scrollLeft = container.scrollTop = 0; layout(); },
        manual(width, height, fit) { manual = { width, height, fit }; layout(); },
        destroy() {
            disposed = true; cancelAnimationFrame(frame); observer.disconnect();
            window.removeEventListener('resize', schedule);
            document.removeEventListener('fullscreenchange', schedule);
            container.removeEventListener('scroll', schedule);
            pixelRatioQuery.removeEventListener('change', pixelRatioChanged);
            root.style.removeProperty('--ivnc-toolbar-height');
        },
    };
}
