// Legacy original-size preferences now select a complete, stretched picture.
export function normalizeDisplayMode(value) {
    if (value === 'native') return 'fill';
    return value === 'fit' || value === 'fill' ? value : null;
}

// One work area for the desktop, input surface and embedded application views.
export function createViewport({ root, container, video, overlay, toolbar, isPinned, isToolbarReserved = isPinned, embedded, onChange }) {
    let disposed = false, frame = 0, mode = 'fit', previousSize = '', pixelRatioQuery;
    let occlusion = { top: 0, inset: 0 };
    const set = (element, property, value) => {
        if (element.style[property] !== value) element.style[property] = value;
    };

    function layout() {
        if (disposed || !container.isConnected) return;
        const fullscreen = document.fullscreenElement;
        // Software keyboards can resize only the visual viewport (not innerHeight
        // or 100dvh). Embedded views already inherit their parent's work area.
        const visual = !embedded && window.visualViewport;
        const unzoomed = visual && Math.abs(visual.scale - 1) < 0.01;
        if (unzoomed) {
            occlusion = {
                top: Math.max(0, visual.offsetTop),
                inset: Math.max(0, window.innerHeight - visual.height - visual.offsetTop),
            };
        } else if (!visual) {
            occlusion = { top: 0, inset: 0 };
        }
        // Pinch zoom alone cannot identify keyboard occlusion. Keep only the
        // known occluded edges, not a cached work-area height: the layout
        // viewport can still resize while zoomed, moving the fixed toolbar.
        const top = Math.min(occlusion.top, window.innerHeight);
        const inset = Math.min(occlusion.inset, window.innerHeight - top);
        const height = window.innerHeight - top - inset;
        root.style.setProperty('--ivnc-visible-height', `${height}px`);
        root.style.setProperty('--ivnc-visible-top', `${top}px`);
        toolbar.style.setProperty('--ivnc-keyboard-inset', `${inset}px`);
        const reservesToolbar = !embedded && isToolbarReserved() && toolbar.isConnected
            && (!fullscreen || fullscreen.contains(toolbar));
        const reserved = reservesToolbar ? Math.min(height, toolbar.getBoundingClientRect().height) : 0;
        root.style.setProperty('--ivnc-toolbar-height', `${reserved}px`);
        const width = container.clientWidth, areaHeight = container.clientHeight;
        let left = 0, videoTop = 0, videoWidth = width, videoHeight = areaHeight;
        const frameWidth = video.videoWidth || Math.max(1, width), frameHeight = video.videoHeight || Math.max(1, areaHeight);
        if (mode === 'fit') {
            const scale = Math.min(width / frameWidth, areaHeight / frameHeight);
            videoWidth = frameWidth * scale; videoHeight = frameHeight * scale;
            left = (width - videoWidth) / 2; videoTop = (areaHeight - videoHeight) / 2;
        }
        for (const element of [video, overlay]) {
            set(element, 'left', `${left}px`); set(element, 'top', `${videoTop}px`);
            set(element, 'width', `${videoWidth}px`); set(element, 'height', `${videoHeight}px`);
        }
        set(video, 'objectFit', mode === 'fill' ? 'fill' : 'contain');
        // Both modes show the complete picture. Focus/legacy scroll offsets
        // must never move its bottom edge underneath the toolbar.
        container.scrollLeft = container.scrollTop = 0;
        const size = `${width}x${areaHeight}@${window.devicePixelRatio}/${frameWidth}x${frameHeight}/${mode}`;
        const changed = size !== previousSize;
        previousSize = size;
        onChange({ width, height: areaHeight, changed });
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
    if (!embedded) {
        window.visualViewport?.addEventListener('resize', schedule);
        window.visualViewport?.addEventListener('scroll', schedule);
    }
    document.addEventListener('fullscreenchange', schedule);
    container.addEventListener('scroll', schedule, { passive: true });
    watchPixelRatio();
    layout();
    return {
        layout,
        caret() {
            schedule();
        },
        mode(value) {
            const next = normalizeDisplayMode(value) || 'fit';
            if (mode !== next) {
                mode = next;
                container.scrollLeft = container.scrollTop = 0;
            }
            layout();
        },
        automatic() { this.mode('fit'); },
        manual(width, height, fit) { this.mode(fit ? 'fit' : 'fill'); },
        destroy() {
            disposed = true; cancelAnimationFrame(frame); observer.disconnect();
            video.removeEventListener('resize', schedule);
            video.removeEventListener('loadedmetadata', schedule);
            window.removeEventListener('resize', schedule);
            window.visualViewport?.removeEventListener('resize', schedule);
            window.visualViewport?.removeEventListener('scroll', schedule);
            document.removeEventListener('fullscreenchange', schedule);
            container.removeEventListener('scroll', schedule);
            pixelRatioQuery.removeEventListener('change', pixelRatioChanged);
            root.style.removeProperty('--ivnc-toolbar-height');
            root.style.removeProperty('--ivnc-visible-height');
            root.style.removeProperty('--ivnc-visible-top');
            toolbar.style.removeProperty('--ivnc-keyboard-inset');
        },
    };
}
