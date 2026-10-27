//! Blocking ads in CutCraft's browser: requests to well-known advertising
//! and tracking networks are refused. Extra domains can be listed in
//! `adblock.txt` in the settings folder - one per line, in the usual
//! formats (`example.com`, `0.0.0.0 example.com`, `||example.com^`), with
//! `#` or `!` starting a comment.

use std::collections::HashSet;

/// Ad and tracking networks blocked by default (and everything under them).
const BUILT_IN: &[&str] = &[
    // Google ads and analytics
    "doubleclick.net", "googlesyndication.com", "googleadservices.com", "google-analytics.com", "googletagmanager.com",
    "googletagservices.com", "adservice.google.com", "imasdk.googleapis.com",
    // ad exchanges and networks
    "amazon-adsystem.com", "adnxs.com", "adsrvr.org", "rubiconproject.com", "pubmatic.com", "openx.net", "criteo.com",
    "criteo.net", "casalemedia.com", "indexww.com", "smartadserver.com", "adform.net", "yieldmo.com", "sharethrough.com",
    "teads.tv", "triplelift.com", "3lift.com", "sovrn.com", "lijit.com", "gumgum.com", "bidswitch.net", "contextweb.com",
    "33across.com", "sonobi.com", "kargo.com", "undertone.com", "emxdgt.com", "onetag-sys.com", "seedtag.com",
    "advertising.com", "serving-sys.com", "flashtalking.com", "adroll.com", "media.net", "yieldlove.com", "smaato.net",
    "adcolony.com", "applovin.com", "unityads.unity3d.com", "vungle.com", "inmobi.com", "mopub.com", "adition.com",
    "adtelligent.com", "e-planning.net", "improvedigital.com", "rhythmone.com", "springserve.com", "spotxchange.com",
    "spotx.tv", "freewheel.tv", "innovid.com", "connatix.com", "aniview.com", "vidazoo.com", "primis.tech", "unrulymedia.com",
    "trafficjunky.net", "exoclick.com", "juicyads.com", "propellerads.com", "popads.net", "popcash.net", "adsterra.com",
    "hilltopads.net", "adcash.com", "clickadu.com", "mgid.com", "revcontent.com", "taboola.com", "outbrain.com",
    "zemanta.com", "nativo.com", "carbonads.com", "carbonads.net", "buysellads.com", "buysellads.net",
    // site ad managers
    "ezoic.net", "ezojs.com", "ezodn.com", "adthrive.com", "mediavine.com", "freestar.io", "pub.network", "cpmstar.com",
    "venatusmedia.com", "playwire.com", "nitropay.com", "snigelweb.com", "adinplay.com", "adngin.com", "fuseplatform.net",
    "setupad.net", "adpushup.com", "monetizemore.com",
    // ad verification and measurement
    "moatads.com", "doubleverify.com", "adsafeprotected.com", "iasds01.com", "scorecardresearch.com", "quantserve.com",
    "quantcount.com", "comscore.com", "chartbeat.com", "chartbeat.net",
    // tracking pixels and analytics
    "hotjar.com", "clarity.ms", "bat.bing.com", "ads-twitter.com", "analytics.twitter.com", "ads.linkedin.com",
    "connect.facebook.net", "mc.yandex.ru", "adsymptotic.com", "crwdcntrl.net", "demdex.net", "everesttech.net",
    "krxd.net", "bluekai.com", "exelator.com", "agkn.com", "rlcdn.com", "tapad.com", "liadm.com", "id5-sync.com",
];

pub struct Blocker {
    domains: HashSet<String>,
}

impl Blocker {
    /// The built-in list plus `adblock.txt` from the settings folder.
    pub fn load() -> Blocker {
        let mut b = Blocker { domains: BUILT_IN.iter().map(|d| d.to_string()).collect() };
        if let Ok(text) = std::fs::read_to_string(crate::prefs::config_dir().join("adblock.txt")) {
            b.add_list(&text);
        }
        b
    }

    /// Adds domains from a block list (hosts file or simple filter lines).
    pub fn add_list(&mut self, text: &str) {
        for line in text.lines() {
            let line = line.split(['#', '!']).next().unwrap_or("").trim();
            // the domain is the last word: "0.0.0.0 ads.example.com" → ads.example.com
            let Some(word) = line.split_whitespace().last() else { continue };
            let d = word.trim_start_matches("||").trim_end_matches('^').trim_start_matches("*.").trim_end_matches('.').to_lowercase();
            if d.contains('.') && !d.contains('/') && !d.contains('*') && d != "0.0.0.0" && d != "127.0.0.1" && d != "localhost" {
                self.domains.insert(d);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.domains.len()
    }

    pub fn is_empty(&self) -> bool {
        self.domains.is_empty()
    }

    /// Whether a request to `url` should be refused.
    pub fn blocks(&self, url: &str) -> bool {
        let Some(host) = host_of(url) else { return false };
        // the host and every domain above it: a.b.ads.com, b.ads.com, ads.com
        let mut h = host.as_str();
        loop {
            if self.domains.contains(h) {
                return true;
            }
            match h.split_once('.') {
                Some((_, rest)) if rest.contains('.') => h = rest,
                _ => return false,
            }
        }
    }

    /// A script for the page that refuses its requests to the blocked
    /// domains, for web views that can't filter requests themselves (macOS,
    /// Linux): fetch, XMLHttpRequest, beacons, and script, frame, picture and
    /// style links set from code or put in the page. Each refusal posts
    /// "blocked" to `window.ipc`.
    pub fn page_script(&self) -> String {
        let mut domains: Vec<&str> = self.domains.iter().map(|d| d.as_str()).collect();
        domains.sort_unstable();
        let list = serde_json::to_string(&domains).unwrap_or_else(|_| "[]".into());
        PAGE_SCRIPT.replace("__DOMAINS__", &list)
    }
}

const PAGE_SCRIPT: &str = r#"(() => {
  const D = new Set(__DOMAINS__);
  const blocked = (u) => {
    let h;
    try { h = new URL(String(u), location.href).hostname.toLowerCase(); } catch (e) { return false; }
    for (;;) {
      if (D.has(h)) return true;
      const i = h.indexOf('.');
      if (i < 0) return false;
      const rest = h.slice(i + 1);
      if (!rest.includes('.')) return false;
      h = rest;
    }
  };
  const note = () => { try { window.ipc.postMessage('blocked'); } catch (e) {} };
  const fetch0 = window.fetch;
  if (fetch0) {
    window.fetch = function (input, init) {
      const u = typeof input === 'string' ? input : input && input.url;
      if (u && blocked(u)) { note(); return Promise.reject(new TypeError('Blocked')); }
      return fetch0.apply(this, arguments);
    };
  }
  const open0 = XMLHttpRequest.prototype.open;
  XMLHttpRequest.prototype.open = function (m, u) {
    if (blocked(u)) { note(); arguments[1] = 'about:blank'; }
    return open0.apply(this, arguments);
  };
  if (navigator.sendBeacon) {
    const beacon0 = navigator.sendBeacon.bind(navigator);
    navigator.sendBeacon = (u, d) => (blocked(u) ? (note(), true) : beacon0(u, d));
  }
  const guard = (C, prop) => {
    const d = C && Object.getOwnPropertyDescriptor(C.prototype, prop);
    if (!d || !d.set) return;
    Object.defineProperty(C.prototype, prop, Object.assign({}, d, {
      set(v) { if (blocked(v)) { note(); return; } d.set.call(this, v); },
    }));
  };
  guard(window.HTMLScriptElement, 'src');
  guard(window.HTMLIFrameElement, 'src');
  guard(window.HTMLImageElement, 'src');
  guard(window.HTMLLinkElement, 'href');
  const setAttr0 = Element.prototype.setAttribute;
  Element.prototype.setAttribute = function (n, v) {
    const k = String(n).toLowerCase();
    if ((k === 'src' || k === 'href') && /^(SCRIPT|IFRAME|IMG|LINK)$/.test(this.tagName) && blocked(v)) { note(); return; }
    return setAttr0.call(this, n, v);
  };
  // whatever arrives in the page's own HTML
  const sweep = (n) => {
    if (n.nodeType !== 1 || !/^(SCRIPT|IFRAME|IMG|LINK)$/.test(n.tagName)) return;
    const u = n.getAttribute('src') || (n.tagName === 'LINK' && n.getAttribute('href'));
    if (u && blocked(u)) { n.remove(); note(); }
  };
  new MutationObserver((ms) => {
    for (const m of ms) for (const n of m.addedNodes) {
      sweep(n);
      if (n.querySelectorAll) n.querySelectorAll('script[src],iframe[src],img[src],link[href]').forEach(sweep);
    }
  }).observe(document, { childList: true, subtree: true });
})();"#;

/// The host name of a web address, lower case.
fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://").map(|(_, r)| r)?;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit('@').next()?;
    let host = host.split(':').next()?;
    (!host.is_empty()).then(|| host.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_ad_networks_and_their_subdomains_only() {
        let mut b = Blocker { domains: BUILT_IN.iter().map(|d| d.to_string()).collect() };
        assert!(b.blocks("https://securepubads.g.doubleclick.net/tag/js/gpt.js"));
        assert!(b.blocks("https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js?client=1"));
        assert!(!b.blocks("https://www.svgrepo.com/show/12345/heart.svg"));
        assert!(!b.blocks("https://www.fontrepo.com/font/7/a-bugs-life.ttf"));
        // look-alikes aren't caught
        assert!(!b.blocks("https://notdoubleclick.net/x"));
        b.add_list("# my list\n0.0.0.0 ads.example.org\n||tracker.example.com^\nplain.example.net ! note\n");
        assert!(b.blocks("http://ads.example.org:8080/a"));
        assert!(b.blocks("https://cdn.tracker.example.com/t.js"));
        assert!(b.blocks("https://plain.example.net/"));
        assert!(!b.blocks("https://example.org/"));
    }

    #[test]
    fn page_script_carries_the_list() {
        let mut b = Blocker { domains: BUILT_IN.iter().map(|d| d.to_string()).collect() };
        b.add_list("ads.example.org");
        let js = b.page_script();
        assert!(!js.contains("__DOMAINS__"));
        assert!(js.contains("\"doubleclick.net\"") && js.contains("\"ads.example.org\""));
    }
}
