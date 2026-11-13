// One work area for the desktop, input surface and embedded application views.
export function createViewport({ root, container, video, overlay, toolbar, isPinned, embedded, onChange }) {
    let disposed = false, frame = 0, mode = 'fit', previousSize = '', pixelRatioQuery;
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
        container.classList.toggle('manual-exact', mode === 'native');
        const width = container.clientWidth, height = container.clientHeight;
        let left = 0, top = 0, videoWidth = width, videoHeight = height;
        const frameWidth = video.videoWidth || width, frameHeight = video.videoHeight || height;
        const scale = mode === 'fit' ? Math.min(width / frameWidth, height / frameHeight) : 1;
        videoWidth = frameWidth * scale; videoHeight = frameHeight * scale;
        if (mode === 'fit') { left = (width - videoWidth) / 2; top = (height - videoHeight) / 2; }
        for (const element of [video, overlay]) {
            set(element, 'left', `${left}px`); set(element, 'top', `${top}px`);
            set(element, 'width', `${videoWidth}px`); set(element, 'height', `${videoHeight}px`);
        }
        set(video, 'objectFit', 'contain');
        const size = `${width}x${height}@${window.devicePixelRatio}/${frameWidth}x${frameHeight}/${mode}`;
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
    video.addEventListener('resize', schedule);
    video.addEventListener('loadedmetadata', schedule);
    window.addEventListener('resize', schedule);
    document.addEventListener('fullscreenchange', schedule);
    container.addEventListener('scroll', schedule, { passive: true });
    watchPixelRatio();
    layout();
    return {
        layout,
        mode(value) {
            mode = value === 'native' ? 'native' : 'fit';
            container.scrollLeft = container.scrollTop = 0;
            layout();
        },
        automatic() { this.mode('fit'); },
        manual(width, height, fit) { this.mode(fit ? 'fit' : 'native'); },
        destroy() {
            disposed = true; cancelAnimationFrame(frame); observer.disconnect();
            video.removeEventListener('resize', schedule);
            video.removeEventListener('loadedmetadata', schedule);
            window.removeEventListener('resize', schedule);
            document.removeEventListener('fullscreenchange', schedule);
            container.removeEventListener('scroll', schedule);
            pixelRatioQuery.removeEventListener('change', pixelRatioChanged);
            root.style.removeProperty('--ivnc-toolbar-height');
        },
    };
}
