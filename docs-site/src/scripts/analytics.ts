// PostHog web analytics (ADR-0066). Added to every page by the integration
// in astro.config.mjs only when siteConfig.mjs has a key and the build is the
// real site, so a build without a key ships none of this. The key and host
// come from the settings tag that components/Analytics.astro writes.
//
// What it counts, and nothing else:
// - a page view on every page load, and a page leave when the reader goes
//   (it carries how long the page stayed open);
// - "download_clicked" on a link to a release file, with the release and
//   platform;
// - "github_link_clicked" on any other link to github.com, with where it
//   points.
//
// What it does not do: store anything on the reader's device (no cookies, no
// localStorage or sessionStorage), make a profile of anyone, record sessions,
// show surveys, capture every click and the text of what was clicked
// (autocapture), load any further script, or ask PostHog for flags or remote
// settings. A browser that sends Do Not Track is not counted.
//
// The slim bundle with no external loading is the smallest build that does
// this: it carries no autocapture, replay or surveys code at all, and cannot
// fetch any. It is bundled into the site's own script, so the page loads no
// third-party script, and its version is pinned in package-lock.json.
import posthog from 'posthog-js/dist/module.slim.no-external.js';

const el = document.getElementById('posthog-config');
const config = el ? (JSON.parse(el.textContent ?? '{}') as { key?: string; host?: string }) : {};

if (config.key && config.host) {
  posthog.init(config.key, {
    api_host: config.host,
    // Nothing stored on the device: PostHog tells visitors apart on its own
    // servers, with a hash of a salt that changes daily, the IP address, the
    // browser and the host (cookieless tracking, which must also be switched
    // on in the PostHog project's settings, or these events are discarded).
    cookieless_mode: 'always',
    persistence: 'memory',
    person_profiles: 'never',
    // Every page is a full page load; there is no client-side router.
    capture_pageview: true,
    capture_pageleave: true,
    autocapture: false,
    rageclick: false,
    capture_dead_clicks: false,
    capture_heatmaps: false,
    capture_performance: false,
    capture_exceptions: false,
    disable_session_recording: true,
    disable_surveys: true,
    disable_external_dependency_loading: true,
    advanced_disable_flags: true,
    respect_dnt: true,
    // UTM and ref= parameters are kept (they say where a visit came from);
    // anything that names a person, such as click IDs, is masked.
    mask_personal_data_properties: true,
  });

  // A link to a release file: polinrider-v3.2.0-macos-arm64.tar.gz is release
  // v3.2.0, platform macos-arm64.tar.gz, the same names the traffic log uses.
  const RELEASE_FILE = /^\/meSingh\/polinrider-(?:scanner|cleaner)\/releases\/download\/([^/]+)\/([^/]+)$/;

  const platformOf = (tag: string, file: string) => {
    if (file === 'SHA256SUMS' || file.endsWith('.intoto.jsonl')) return 'checksums and attestations';
    const at = file.indexOf(tag);
    return (at === -1 ? file : file.slice(at + tag.length)).replace(/^[-.]+/, '') || file;
  };

  // One listener for the whole page, so every link is covered without any
  // markup of its own. Sent at once, by beacon, so it is not lost when the
  // click opens a new tab or starts a download.
  document.addEventListener(
    'click',
    (event) => {
      const link = (event.target as Element | null)?.closest?.('a[href]');
      if (!(link instanceof HTMLAnchorElement)) return;
      let url: URL;
      try {
        url = new URL(link.href);
      } catch {
        return;
      }
      if (url.hostname !== 'github.com') return;
      const send = { send_instantly: true, transport: 'sendBeacon' } as const;
      const file = RELEASE_FILE.exec(url.pathname);
      if (file) {
        const [, tag, name] = file;
        posthog.capture('download_clicked', { tag, file: name, platform: platformOf(tag, name) }, send);
      } else {
        posthog.capture('github_link_clicked', { target: `github.com${url.pathname}` }, send);
      }
    },
    { capture: true },
  );
}
