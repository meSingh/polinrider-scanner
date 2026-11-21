# 0066. Send the site's and GitHub's usage numbers to PostHog, on its EU cloud

| | |
|---|---|
| **Status** | Accepted; host amended by [ADR-0067](./0067-the-posthog-project-is-on-its-us-cloud.md) |
| **Date** | 2026-11-21 |

## Context

The site's analytics have changed twice. Plausible was planned and dropped as
paid on 2026-11-04. Since then the site has loaded Cloudflare Web Analytics'
JS beacon (ADR-0055 carried the switch over), which counts page views without
cookies but keeps them inside Cloudflare, and takes nothing else: there is no
way to send it other data. Its GraphQL API can read the site's numbers out,
so the other way round was considered (pull them into the `traffic` branch
and draw a dashboard there), and not chosen.

GitHub's numbers are on the `traffic` branch (ADR-0053): views, clones,
referrers, paths, stars and release downloads, with our own kept apart.
Nothing recorded the Homebrew tap's, the Scoop bucket's or the action's
traffic, or how many repositories use the action. And nothing put any of it
beside the website's numbers, which is where the questions are asked: did a
post bring people, and did they download anything.

The maintainer chose PostHog on 2026-11-21. It takes events from a browser
and from any script, keeps them for good on its free tier, and has an EU
region.

The site belongs to a security tool for people who fear their machine is
compromised. What it asks of a visitor's browser matters more than how much
it learns.

## Decision

**The site** loads posthog-js, bundled into the site's own script (the slim
build with no external loading, 1.447.3 pinned in `package-lock.json`), and
sends to `eu.i.posthog.com`:

- cookieless (`cookieless_mode: "always"`) with memory persistence: nothing is
  written to cookies, localStorage or sessionStorage. PostHog tells visitors
  apart on its servers with a hash of a daily salt, the IP address, the user
  agent and the host;
- no person profiles, no session recording, no surveys, no heatmaps, no
  flags or remote config request, no further scripts; Do Not Track is
  respected;
- page views, page leaves, and two named clicks found by one listener:
  `download_clicked` (tag, file, platform) and `github_link_clicked`
  (target).

**Autocapture is off.** It would send every click with the element's text,
classes and link, which is more than the two questions above need, and the
slim build without it is smaller. A third click worth counting is added as a
named event.

Nothing is injected until the build has `PUBLIC_POSTHOG_KEY`, a repository
variable; a value that is not a PostHog project key fails the build. Without
it the build carries no PostHog code at all. The footer and the About page
say what is collected only while it is on. Cloudflare Web Analytics' beacon
is removed.

**GitHub's numbers** stay on the `traffic` branch, which remains the
permanent record. The Traffic log also records the tap's, the bucket's and
the action's views and clones (under `repos/`) and the count of public
repositories using the action (`action-users.csv`), then sends the branch's
numbers to PostHog's capture API (`/batch/`) as events, one per day per
thing, with the day as the timestamp, the distinct_id
`polinrider-traffic-log` and `$process_person_profile: false`
(`.github/scripts/posthog.py`). Each event's uuid is derived from its name,
its day and what it counts, so sending it again replaces it rather than
counting it twice; `posthog/sent.csv` on the branch records what was sent,
so only new or changed events go. A manual run with `posthog_backfill` sends
the whole history. Without the `POSTHOG_PROJECT_KEY` secret the step says
so and passes.

Release downloads per day use the same arithmetic as ADR-0053's weekly
tables, so the two always agree: our own taken off, checksums and
attestations as their own event, and what was counted before the log began
as one event per release and platform on its first day.

This supersedes the analytics part of ADR-0055 and adds to ADR-0053.

## Consequences

Better:

- One place for the site and GitHub, where a visit, a download click and the
  downloads GitHub counted can be read on one timeline.
- The visitor's device keeps nothing, and the page loads no third-party
  script.
- The tap, the bucket and the action's traffic, and the action's users, are
  recorded at all.
- PostHog is replaceable. The branch has every GitHub number; removing the
  site's analytics is unsetting one variable.

Worse:

- **A third party sees every visit.** PostHog receives each visitor's IP
  address and user agent to make the daily hash, even though it does not
  store them with the event. Cloudflare saw the same before; a visitor who
  wants neither has to block the request.
- **Cookieless tracking has to be switched on in the PostHog project**
  (Settings, Web analytics). Until it is, PostHog discards the site's
  events, and says so only as an ingestion warning.
- Visitors are told apart within a day, not across days: there is no count
  of returning visitors and no unique visitors over a week. That is the
  price of storing nothing on the device.
- Browsers that send Do Not Track, and blockers that know PostHog's host,
  are not counted, so the site's numbers are a floor.
- Every page loads about 53 kB (gzip) more script while analytics is on, and
  the site depends on one more npm package, pinned, whose updates need the
  same care as any other.
- De-duplication in PostHog is eventual: after a revised count is resent,
  the old and new event can both show for a while. The ledger keeps this to
  events whose numbers changed.
- `github_top_referrer` and `github_top_path` are GitHub's 14-day totals as
  read each day. Summed across days they count the same visits many times;
  only a single day's, or the latest, means anything.
- Action users come from code search, which needs a user's token
  (`TRAFFIC_TOKEN`) and sees only what GitHub has indexed: a floor.
- The site's history in Cloudflare Web Analytics, from 2026-11-04, is not
  moved to PostHog. It stays in Cloudflare until that is switched off.

## Related

- ADR-0053, the `traffic` branch and the download arithmetic this reuses.
- ADR-0055, which carried the analytics switch over when the site moved to
  plain Astro; its analytics choice is superseded here.
