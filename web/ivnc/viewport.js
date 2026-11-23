// Legacy original-size preferences now select a complete, stretched picture.
export function normalizeDisplayMode(value) {
    if (value === 'native') return 'fill';
    return value === 'fit' || value === 'fill' ? value : null;
}

// One work area for the desktop, input surface and embedded application views.
export function createViewport({ root, container, video, overlay, toolbar, isPinned, isToolbarReserved = isPinned, embedded, onChange }) {
    let disposed = false, frame = 0, mode = 'fit', previousSize = '', pixelRatioQuery;
    let occlusion = { top: 0, inset: 0, left: 0, right: 0 };
    let caretRect = null, caretDesktop = null, caretActive = false, appliedShift = 0;
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
                left: Math.max(0, visual.offsetLeft || 0),
                right: Math.max(0, window.innerWidth - (visual.width || window.innerWidth) - (visual.offsetLeft || 0)),
            };
        } else if (!visual) {
            occlusion = { top: 0, inset: 0, left: 0, right: 0 };
        }
        // Pinch zoom alone cannot identify keyboard occlusion. Keep only the
        // known occluded edges, not a cached work-area height: the layout
        // viewport can still resize while zoomed, moving the fixed toolbar.
        const top = Math.min(occlusion.top, window.innerHeight);
        const inset = Math.min(occlusion.inset, window.innerHeight - top);
        const height = window.innerHeight - top;
        const visibleLeft = Math.min(occlusion.left, window.innerWidth);
        const right = Math.min(occlusion.right, window.innerWidth - visibleLeft);
        const visibleWidth = window.innerWidth - visibleLeft - right;
        root.style.setProperty('--ivnc-visible-height', `${height}px`);
        root.style.setProperty('--ivnc-visible-top', `${top}px`);
        root.style.setProperty('--ivnc-visible-width', `${visibleWidth}px`);
        root.style.setProperty('--ivnc-visible-left', `${visibleLeft}px`);
        toolbar.style.setProperty('--ivnc-keyboard-inset', `${inset}px`);
        toolbar.style.setProperty('--ivnc-visible-width', `${visibleWidth}px`);
        toolbar.style.setProperty('--ivnc-visible-left', `${visibleLeft}px`);
        const reservesToolbar = !embedded && isToolbarReserved() && toolbar.isConnected
            && (!fullscreen || fullscreen.contains(toolbar));
        const reserved = reservesToolbar ? Math.min(height, toolbar.getBoundingClientRect().height) : 0;
        root.style.setProperty('--ivnc-toolbar-height', `${reserved}px`);
        const width = container.clientWidth, areaHeight = container.clientHeight;
        let left = 0, videoTop = 0, videoWidth = width, videoHeight = areaHeight;
        const frameWidth = video.videoWidth || Math.max(1, width), frameHeight = video.videoHeight || Math.max(1, areaHeight);
        const scale = Math.min(width / frameWidth, areaHeight / frameHeight);
        const sourceWidth = frameWidth * scale, sourceHeight = frameHeight * scale;
        if (mode === 'fit') {
            videoWidth = sourceWidth; videoHeight = sourceHeight;
            left = (width - videoWidth) / 2; videoTop = (areaHeight - videoHeight) / 2;
        }
        const presentedHeight = mode === 'fill' ? areaHeight : videoHeight;
        const shift = caretShift(videoTop, presentedHeight, areaHeight, inset, reserved);
        if (shift !== appliedShift) {
            set(container, 'transition', window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'none' : 'transform .18s ease');
            appliedShift = shift;
        }
        for (const element of [video, overlay]) {
            set(element, 'left', `${left}px`); set(element, 'top', `${videoTop}px`);
        }
        set(overlay, 'width', `${videoWidth}px`); set(overlay, 'height', `${videoHeight}px`);
        set(video, 'width', `${sourceWidth}px`); set(video, 'height', `${sourceHeight}px`);
        set(video, 'objectFit', mode === 'fill' ? 'fill' : 'contain');
        // Safari's native MediaStream layer can preserve the source aspect
        // despite object-fit:fill, cropping a wide stream on a portrait iPad.
        // Keep the media box proportional, then stretch the whole layer. Its
        // bounding rect still matches the overlay, including input/caret math.
        // The untransformed box also fits inside the area, so it adds no scroll.
        if (mode === 'fill' && sourceWidth > 0 && sourceHeight > 0) {
            set(video, 'transformOrigin', '0 0');
            set(video, 'transform', `scale(${width / sourceWidth}, ${areaHeight / sourceHeight})`);
        } else {
            set(video, 'transform', 'none');
            set(video, 'transformOrigin', '');
        }
        container.style.transform = shift ? `translateY(${-shift}px)` : 'none';
        container.scrollLeft = container.scrollTop = 0;
        const size = `${width}x${areaHeight}@${window.devicePixelRatio}/${frameWidth}x${frameHeight}/${mode}`;
        const changed = size !== previousSize;
        previousSize = size;
        onChange({ width, height: areaHeight, changed });
    }
    function caretShift(originY, boxHeight, areaHeight, inset, reserved) {
        if (!caretActive || inset < 80 || !caretRect || !caretDesktop) return 0;
        const bottom = originY + ((caretRect[1] + caretRect[3]) / caretDesktop.height) * boxHeight;
        const limit = areaHeight - inset - reserved - 8;
        return Math.max(0, Math.min(Math.max(0, areaHeight - 48), bottom - limit));
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
        caret(rect, desktop, active = false) {
            caretRect = Array.isArray(rect) && rect.length >= 4 ? rect : null;
            caretDesktop = desktop?.width > 0 && desktop?.height > 0 ? desktop : null;
            caretActive = !!active;
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
            root.style.removeProperty('--ivnc-visible-width');
            root.style.removeProperty('--ivnc-visible-left');
            toolbar.style.removeProperty('--ivnc-keyboard-inset');
            toolbar.style.removeProperty('--ivnc-visible-width');
            toolbar.style.removeProperty('--ivnc-visible-left');
        },
    };
}
