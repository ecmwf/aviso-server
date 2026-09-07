(() => {
    function openHashTarget() {
        const target = document.getElementById(location.hash.slice(1));
        const panel = target?.closest(".ecpds-settings details.setting-panel");
        if (panel) {
            panel.open = true;
            target.scrollIntoView();
        }
    }

    window.addEventListener("hashchange", openHashTarget);
    openHashTarget();
})();
