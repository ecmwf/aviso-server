(function () {
    var DARK_THEMES = ['coal', 'navy', 'ayu'];

    function isDark() {
        var classes = document.documentElement.classList;
        return DARK_THEMES.some(function (t) { return classes.contains(t); });
    }

    function applyLogo() {
        var dark = isDark();
        document.querySelectorAll('.logo-light').forEach(function (el) {
            el.style.display = dark ? 'none' : 'block';
        });
        document.querySelectorAll('.logo-dark').forEach(function (el) {
            el.style.display = dark ? 'block' : 'none';
        });
    }

    // Apply immediately on load
    document.addEventListener('DOMContentLoaded', applyLogo);

    // React to theme changes (mdBook toggles class on <html>)
    var observer = new MutationObserver(applyLogo);
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] });
})();
