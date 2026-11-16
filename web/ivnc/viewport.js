// One work area for the desktop, input surface and embedded application views.
export function createViewport({ root, container, video, overlay, toolbar, isPinned, isToolbarReserved = isPinned, embedded, onChange }) {
    let disposed = false, frame = 0, mode = 'fit', previousSize = '', pixelRatioQuery;
    let caret = null, scrollBounds = null, occlusion = { top: 0, inset: 0 };
    const set = (element, property, value) => {
        if (element.style[property] !== value) element.style[property] = value;
    };

    function layout() {
        if (disposed || !container.isConnected) return;
        // Remember an edge before changing the work area. Native-size users at
        // the bottom must still see the last row when the toolbar grows/shows.
        const atRight = mode === 'native' && scrollBounds?.x > 0 && container.scrollLeft >= scrollBounds.x - 1;
        const atBottom = mode === 'native' && scrollBounds?.y > 0 && container.scrollTop >= scrollBounds.y - 1;
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
        container.classList.toggle('manual-exact', mode === 'native');
        const width = container.clientWidth, areaHeight = container.clientHeight;
        let left = 0, videoTop = 0, videoWidth = width, videoHeight = areaHeight;
        const frameWidth = video.videoWidth || Math.max(1, width), frameHeight = video.videoHeight || Math.max(1, areaHeight);
        const scale = mode === 'fit' ? Math.min(width / frameWidth, areaHeight / frameHeight) : 1;
        videoWidth = frameWidth * scale; videoHeight = frameHeight * scale;
        if (mode === 'fit') { left = (width - videoWidth) / 2; videoTop = (areaHeight - videoHeight) / 2; }
        for (const element of [video, overlay]) {
            set(element, 'left', `${left}px`); set(element, 'top', `${videoTop}px`);
            set(element, 'width', `${videoWidth}px`); set(element, 'height', `${videoHeight}px`);
        }
        set(video, 'objectFit', 'contain');
        if (atRight) container.scrollLeft = container.scrollWidth;
        if (atBottom) container.scrollTop = container.scrollHeight;
        if (mode === 'native' && caret?.follow && caret.desktop?.width > 0 && caret.desktop?.height > 0) {
            const [x, y, w, h] = caret.rect;
            const sx = videoWidth / caret.desktop.width, sy = videoHeight / caret.desktop.height;
            const right = (x + w) * sx + 12, bottom = (y + h) * sy + 12;
            const desiredX = x * sx - 12, desiredY = y * sy - 12;
            if (desiredX < container.scrollLeft) container.scrollLeft = Math.max(0, desiredX);
            else if (right > container.scrollLeft + width) container.scrollLeft = right - width;
            if (desiredY < container.scrollTop) container.scrollTop = Math.max(0, desiredY);
            else if (bottom > container.scrollTop + areaHeight) container.scrollTop = bottom - areaHeight;
        }
        scrollBounds = { x: container.scrollWidth - container.clientWidth, y: container.scrollHeight - container.clientHeight };
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
        caret(rect, desktop, follow) {
            caret = Array.isArray(rect) && rect.length === 4 && rect.every(Number.isFinite) ? { rect, desktop, follow } : null;
            schedule();
        },
        mode(value) {
            const next = value === 'native' ? 'native' : 'fit';
            if (mode !== next) {
                mode = next;
                scrollBounds = null;
                container.scrollLeft = container.scrollTop = 0;
            }
            layout();
        },
        automatic() { this.mode('fit'); },
        manual(width, height, fit) { this.mode(fit ? 'fit' : 'native'); },
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
