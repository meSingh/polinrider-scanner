# 0067. The PostHog project is on PostHog's US cloud

| | |
|---|---|
| **Status** | Accepted |
| **Date** | 2026-11-21 |

## Context

ADR-0066 sent the site's page views and the Traffic log's numbers to
PostHog's EU cloud, `eu.i.posthog.com`. The project that holds the key was
created on PostHog's US cloud. Each cloud keeps its own projects, and an
event sent to the other one with a key it does not know is answered with
HTTP 200 and dropped. So the first day's page views and the 1,075 events of
the Traffic log's first run reached nothing, while every check that looked at
the response said they had been sent.

Mandeep chose to keep the US project rather than create one on the EU cloud
(2026-11-21).

## Decision

The site and the Traffic log send to `us.i.posthog.com`. Everything else in
ADR-0066 stands: cookieless, nothing stored on the visitor's device, no
person profiles, no recording, Do Not Track respected, and the `traffic`
branch as the permanent record.

The privacy text no longer names a region: the footer and the About page say
PostHog counts, not where.

The history is sent again with the Traffic log's backfill, which replaces
rather than double counts.

## Consequences

- Visitor data is processed in the US. PostHog's cookieless processing keeps
  no IP address or user agent with the events.
- A 200 from PostHog does not prove an event arrived. The project's own
  "ingested event" setting, or the events list, does. Check that after any
  change of key or host.
- Cookieless tracking must be on in the project's web analytics settings, or
  PostHog discards every site event. It was off when this was found.

## Related

- ADR-0066: amended here, the host only.
