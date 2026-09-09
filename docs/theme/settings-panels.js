(() => {
    function openHashTarget(hash = location.hash) {
        let id;
        try {
            id = decodeURIComponent(hash.slice(1));
        } catch {
            return;
        }
        const target = document.getElementById(id);
        const panel = target?.closest(".settings-reference details.setting-panel");
        if (panel) {
            panel.open = true;
            target.scrollIntoView();
        }
    }

    // A repeated index link does not fire hashchange after its panel is closed.
    document.addEventListener("click", (event) => {
        const link = event.target.closest("a[href]");
        if (!link || event.button !== 0 || event.ctrlKey || event.metaKey ||
            event.shiftKey || event.altKey || link.hasAttribute("download") ||
            (link.target && link.target !== "_self")) return;
        const url = new URL(link.href);
        if (url.origin === location.origin && url.pathname === location.pathname &&
            url.search === location.search && url.hash === location.hash) {
            openHashTarget(url.hash);
        }
    });
    window.addEventListener("hashchange", () => openHashTarget());
    openHashTarget();
})();
