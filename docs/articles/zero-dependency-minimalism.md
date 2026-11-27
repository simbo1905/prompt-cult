# Zero dependency minimalism: somebody finally said it out loud

*Draft article — seed for a series on the zero-dependency / no-framework
position. Written 27 November 2026, the day Fireship published
["The wild world of tech in 2027..."](https://youtu.be/sPGzM3ktJVE).*

For years, "zero dependencies" sounded like a personality trait. You said it
and people heard: this person hand-rolls date parsing and thinks the browser
is a framework. Then on 27 November 2026, Fireship — a channel with an
audience measured in millions, whose job is to report what is mainstream —
put it on the board as a named trend: **zero dependency minimalism**. Not a
joke aside, not a fringe subculture. One of the ten trends developers need
to watch.

This article is the working document behind my claim to have been saying
this for a long time, and the foundation for a series of pieces about why
the AI era makes fewer dependencies not just virtuous but structurally
necessary. The working artefact behind the claim is
[vanilla-pod-js](https://prompt-cult.github.io/prompt-cult/), a published
skill and green fixture for framework-free browser JavaScript with zero
runtime dependencies — built before "zero dependencies" was a trend anyone
needed a video for. Behind the skill sits a longer public record: a trail
of gists documenting entire front-end architectures built this way, in
production, which this article mines for evidence.

## What the video actually says

The relevant passage, from the auto-generated transcript of
["The wild world of tech in 2027..."](https://youtu.be/sPGzM3ktJVE)
(Fireship, "The Code Report", published 27 November 2026). The segment sits
between the "disposable software" trend and the "year of the agent" recap:

> And in similar fashion, many developers are embracing zero dependency
> minimalism. Out on the street, you don't really hear people fighting
> about Next.js versus whatever other meta framework anymore. And there's
> a reason for that. Running npm install nowadays is riskier than riding
> on a Spanish bullet train. [...] This year alone, npm got hit by [a]
> canister worm in March, [and Shai-Hulud] in May, which poisoned over 170
> packages, including TanStack, and created a chain reaction that affected
> hundreds of packages. It got so bad that npm 12 now refuses to run
> install scripts by default. And that's not all. You also have to
> consider slop squatting, where attackers register fake package names
> that LLMs hallucinate, because one study found that nearly 20% of AI
> recommended packages don't even exist at all. So instead of installing
> 100 npm packages like we used to do in the old days, many devs are now
> just generating their own utilities on the fly. [...] And the reality is
> that 90% of problems out there can be solved with vanilla JavaScript,
> Python, and Rust. Those are the languages the LLMs are best at. And that
> means those are the languages that will continue to dominate in the
> future.

Three claims packed into one segment, and each deserves unpicking:

1. **The framework wars went quiet.** Nobody fights about meta-frameworks
   any more because the marginal value of the tenth abstraction stopped
   justifying its supply chain.
2. **`npm install` became the attack surface.** The registry is now the
   single most effective malware distribution channel in software.
3. **AI inverted the economics.** When an LLM can generate a small utility
   on demand, the hundred-package `node_modules` folder stops being free
   and starts being a liability you chose.

## The receipts behind the jokes

The video is comedy, but every load-bearing fact in that segment is real
and checkable:

- **Shai-Hulud, September 2025.** The first self-replicating worm in the
  npm ecosystem: it stole maintainer credentials, then used them to
  publish trojanised versions of the victims' own packages, propagating
  with no human in the loop. [CISA issued an alert](https://www.cisa.gov/news-events/alerts/2025/09/23/widespread-supply-chain-compromise-impacting-npm-ecosystem)
  after 500+ packages were compromised. [Unit 42's write-up](https://unit42.paloaltonetworks.com/npm-supply-chain-attack/)
  traces the mechanism; npm never fully recovered its reputation from it.
- **Shai-Hulud 2.0, November 2025.** The second coming: [Datadog counted 796 backdoored packages](https://securitylabs.datadoghq.com/articles/shai-hulud-2.0-npm-worm/)
  and tens of thousands of developer repositories exposing secrets. The
  [Unit 42 threat-landscape review](https://unit42.paloaltonetworks.com/monitoring-npm-supply-chain-attacks/)
  treats post-Shai-Hulud as *the new baseline*, with further campaigns
  through 2026 — which is the attack wave the video's "in May" line
  gestures at.
- **npm 12 refuses to run install scripts by default.** Lifecycle scripts
  were the worm's fuel; the registry's own tooling now treats them as
  guilty until allow-listed. When the default installer stops executing
  other people's code, the ecosystem is admitting the threat model out
  loud.
- **Slopsquatting is peer-reviewed.** The term was coined by Seth Larson
  at the Python Software Foundation. The number behind "nearly 20%" is the
  [USENIX Security 2025 study "We Have a Package for You!"](https://arxiv.org/abs/2504.08573)
  (Spracklen et al.): 576,000 code samples across 16 models, 19.7% of
  samples referencing at least one package that does not exist, 205,474
  unique invented names — and hallucinated names *repeat*, which is what
  makes pre-registering them a viable business model for attackers. An
  agent that runs `npm install` without a human reading the name completes
  the attack with zero interaction.

So the trend is not vibecoded Vibes. It is the rational response of an
industry that watched a self-replicating worm crawl its dependency graph
and decided the graph was optional.

## Why AI changes the dependency calculus

The interesting part is not "npm had a bad year". Dependency counts were
already a smell. The interesting part is that AI breaks the trade-off that
justified dependencies for twenty years.

**The old bargain:** writing and maintaining your own utility costs
engineering time; a dependency costs a `npm install`. Dependencies won
because human labour was the scarce resource.

**The new scarcity is verification, not generation.** Code production is
now nearly free and effectively unbounded — the same video's first trend
is a job market drowning in AI-generated code that somebody must maintain.
When generation is cheap and review is the bottleneck, every dependency is
a deferred review debt: transitive code you did not write, running with
your permissions, executed at install time, re-evaluated on every lockfile
change. The unit of trust got smaller, and dependencies are the way you
maximise the number of units you must trust.

**The model is the framework now.** The video's closing line of the
segment is the deepest one: the languages the LLMs are best at — vanilla
JavaScript, Python, Rust — are the ones that will dominate. The training
corpus is the framework. A pattern that appears ten million times in the
corpus is generated correctly, idiomatically, and reviewably by any model;
a pattern that lives in this month's meta-framework is generated from a
thinner sample and reviewed against documentation that may not have been
in the training data at all. Convention beats novelty not because
convention is aesthetically pleasing but because it is statistically
better supported.

**Agents make the dependency graph a hallucination surface.** Every
dependency you permit is a name an agent may invent and install.
Slopsquatting converts your dependency budget directly into an attack
surface. Zero dependencies is the only configuration where the model
cannot hallucinate a package into your build, because there is no install
step to hallucinate into.

**Small, inspectable code is agent-legible.** A few hundred lines of plain
JavaScript fit in a context window. An agent can read the whole thing,
verify the whole thing, and refactor the whole thing without a summarised
mental model of somebody else's architecture. In an AI-assisted workflow,
readable-in-one-sitting is a performance feature.

Put together: **the dependencies you don't have can't be wormed, can't be
hallucinated, can't be yanked, and don't need to be reviewed.** Less is
more stops being a taste and becomes a threat model.

## Vanilla Pod JS: the working answer

This is not a new position for me, and it is not only a position — it ships.
[vanilla-pod-js](https://prompt-cult.github.io/prompt-cult/skills/vanilla-pod-js/SKILL.md)
is a published skill in the Prompt Cult catalogue: framework-free browser
JavaScript with **zero runtime dependencies**, with a green fixture that
proves the whole story in CI. Its pillars line up one-to-one with the
threat model above:

- **RFC 8927 JSON Type Definition schemas, compiled ahead of time.**
  [jtd-codegen](https://github.com/prompt-cult/json-type-definition-RFC-8927)
  compiles a schema into a validator module that *imports nothing*. The
  validation logic exists at build time, is generated from a standard (the
  RFC beats test, doc, and code when they disagree), and is committed to
  the repository — so the browser runs plain, readable, dependency-free
  ES modules with no bundler in the path and no runtime validation library
  to trust, update, or get wormed.
- **Validate before use, fail closed.** The fetcher kernel consumes NDJSON
  row streams: parse, validate, freeze, yield. The first bad row throws
  with that row's diagnostics. Nothing invalid is dropped, yielded, or
  coerced; an empty stream is a valid answer and null never is. This is
  the "don't trust the input" discipline of the supply-chain world applied
  to the data path.
- **Frozen immutability at the trust boundary.** Everything crossing the
  wire is deeply frozen right after validation. Downstream code physically
  cannot mutate a wire row; mutations become new values created by pure
  projections. Data you have verified stays verified.
- **Type checking without transpiling.** TypeScript runs as a checker only
  (`tsc --noEmit` over JSDoc-annotated `.mjs`). There are no `.ts` files,
  no emit step, no source maps: the browser executes the exact bytes the
  human wrote and the agent read. The build you don't have can't break,
  and the "what did the compiler do to my code" class of debugging simply
  does not exist.
- **A check gate, not a demo.** `make check` runs codegen from the real
  compiler, `node --check` on every module, the test suite, and the type
  check over the whole tree. Four tools total; no `node_modules` in the
  runtime path.

The deeper point: vanilla-pod-js does not recover safety by *trusting less
software* — it recovers safety by *generating the minimum software the
contract requires and auditing all of it*. The schema is data, the
generated validator is derived, and the handwritten glue is small enough
to read in one sitting. That is what "trust" looks like when generation is
cheap: not more packages, but fewer, smaller, derived-from-spec files.

The same discipline runs through the rest of the constellation: the
[no-framework-pickler](https://github.com/simbo1905/no-framework-pickler)
applies it to Java serialisation, the lua-lunet tooling applies it to
systems tooling, and the Prompt Cult proxy spec applies it to credentials —
one process holds the keys so the agent harness holds none. Fewer things
held, fewer things that can leak. It is one idea wearing different coats.

## Built, not theorised: the gist record

The skill is the consolidated artefact, but the thinking did not start
there. My public gists are the working record of several years of building
entire front-end architectures this way — long before a mainstream channel
gave the position a name. Three of them carry the core of the argument and
deserve to be read as prior art on myself:

- [vanilla-pod-js — framework-free typed browser JS](https://gist.github.com/simbo1905/5be015475abd1a1611c0e996218ce4e7)
  (now a pointer to the skill; the revision history is the trail),
- [Functional fetchers with vanilla JS, uv, JTD, and typed views](https://gist.github.com/simbo1905/e9ff1159497ddfaad195a224e8fb4a3d),
- [Tripwire API — honest current pattern](https://gist.github.com/simbo1905/c5645488616d0374936659f798aeea68).

### The problem the gists named first

Long before "zero dependency minimalism" was a trend, the gists framed the
exact failure the trend reacts to — and it is not mainly a supply-chain
failure. It is a **trust failure at the runtime boundary**:

> TypeScript sells you compile-time checking and then makes you transpile,
> ship `.d.ts`, and fight a build system — all to end up with types that
> **vanish at runtime**, exactly where untrusted JSON arrives. The usual
> alternatives (Zod, io-ts) add a runtime dependency to reinvent what a
> JSON type system already does.

Two sources of truth, a build tax, and an archaeology dig through
declaration files — and at the one moment where the data is actually
hostile, the type system has already gone home. The response in the gists
was not "add a validation library". It was: the type system for JSON
already exists, it is a standard (RFC 8927), so *generate* the runtime
checks from it and delete the second source of truth.

### The boundary equation

The functional-fetchers gist states the whole architecture as one
invariant:

```text
database view == JTD == generated validator == generated JSDoc
```

One field-name invariant, four artefacts, no framework anywhere in it. The
view is proven when the migration runs; the schema is data; the validator
and the JSDoc types are compiled artifacts of that schema; `tsc --noEmit`
checks the handwritten code against the generated types; the runtime
validator checks the wire against the same schema before anything is
frozen and handed to the app. The code says exactly where trust begins:

```text
before validation: unknown
after validation:  frozen, validated Row
```

Everything downstream of that line is ordinary functions — `map`,
`flatMap`, `collect` over async generators, a full-arity kernel whose
dependencies are bound explicitly in lifecycle order (`Config →
Observability → FetchAssembly → RunContext → Predicate`). That last part
matters for the zero-dependency argument: the thing a dependency-injection
container does is here done by currying, in maybe twenty lines you can
read in one sitting. As that gist puts it:

> The architecture is not the framework. The architecture is the visible
> shape of the computation.

### Proven in production, twice over

This is not a thought experiment. The Tripwire gist documents the pattern
running end-to-end in a production financial system: an Open Data Contract
YAML drives generation of the SQL view, the JTD schema, the JS validator,
the fetcher, the JSDoc types, *and* the Python serialisation — one
contract, five generated artefacts, two languages — with deliberately
boring handwritten adapters at the edges and a Makefile lane proving the
boundary on every build. The margin-monitor screen served real users
through a chain of `Postgres relation → generated view SQL → thin adapter →
functional fetcher → generated validator → frozen rows`. And the LSE AI
Leadership course site shipped a 549-page corpus with AOT-generated
validators guarding the chat tool-call surface, JTD-validated ingest, and
search over a WASM index — no framework anywhere, `tsc --noEmit` clean.

Both write-ups follow the same honesty rule, which is itself part of the
discipline: the source of truth is the code and the generated artifacts;
the write-up is a map, not a substitute for reading the files. Zero
dependencies does not mean zero evidence.

## Late arrivals: state without a state machine

The same trail contains the second half of the front-end argument. The
taxing part of SPA state management was never rendering — it was the
ceremonies grown around keeping views in sync: stores, reducers, selectors,
hydration, context providers, and the whole menagerie of "toxic stuff" that
exists to answer one question: *what about the events that happened before
I was listening?*

Open a tab late — or materialise a component late — and every message the
page would have cared about has already been dispatched. BroadcastChannel,
by specification, keeps no backlog: a post reaches only the contexts
listening at dispatch time. Most architectures respond by bolting on a
central store to replay state. The
[late-arriver](https://github.com/prompt-cult/late-arriver) demo in the
Prompt Cult org takes the other route: make the *bus itself* durable, and
late arrival stops being a special case.

The demo is deliberately humble: a **logging console**. Everything a page
does gets published to the bus; the console is a drop-in Web Component you
can add to any page. Open it while things are happening and it shows the
log live; open it later — even though the developer tools were never open —
and it catches up on everything it missed, exactly once, then follows the
live stream. That is the whole demonstration: a Web Component that mounts
late, finds the state that has already been spooled, and then listens on
the event bus for what comes next. It is the smallest possible instance of
the general claim that any component on any screen can (a) find the state
that has already arrived and (b) subscribe to the event bus for what comes
next, from the backend or from other pages. The console is just the
smallest useful instance of that claim; every screen's components can be
the same kind of citizen.

The architecture is four small pieces:

1. **Producers** post envelopes to a raw `BroadcastChannel`. They never
   touch storage.
2. **A spool worker** (a Web Worker) writes every envelope to IndexedDB in
   a readwrite transaction, and re-broadcasts it on a second channel from
   inside `tx.oncomplete` — persist-then-broadcast, literally, for
   everything the UI ever sees.
3. **A late subscriber** boots in a fixed order: subscribe first, then read
   the IndexedDB backlog snapshot, then merge, then go live. For any entry,
   exactly one of two things is true — it committed before the snapshot
   (it is in the backlog) or after it (its re-broadcast arrives after the
   subscription existed, so it is buffered). The only overlap mode is
   both, and dedupe-by-id absorbs it. Neither case can be empty, so
   nothing is silently dropped.
4. **Validators on the bus.** Every envelope is shaped by an RFC 8927
   schema and checked by the generated validator; the barrel of validators
   lets any component subscribe to exactly the message types it declares.
   Persisted state in IndexedDB/session storage is validated on load with
   the same generated modules — storage is just another untrusted wire.

The correctness claim is a property of the design, not of vigilance, and
the README proves it from the IndexedDB and BroadcastChannel
specifications: the only duplicate mode is "committed in both windows",
and dedupe-by-id absorbs it; reversing the boot order reopens a gap, which
is why the boot order is fixed and the readiness handshake exists. The
measured behaviour: 23 entries emitted with no console open were caught
23/23 by a console mounted afterwards; a subscriber booted mid-stream
reported `1725 from backlog, 9 buffered during read` and deduped to
exactly 1725; three subscribers mounted at three different times each held
a gap-free, duplicate-free sequence over a 3,363-entry run.

Compare that to what the framework world offers for the same problem —
stores, reducers, selectors, hydration, context providers, and the entire
menagerie of state-sync ceremony whose full-time job is answering "what
about the events that happened before I was listening?". A durable bus and
a total order over entries answer the same question with two channels, one
worker, and one pure merge function that is unit-tested, including a
property test over the interleavings the invariant permits.

And it composes with the rest of the stack: the envelopes on the bus are
validated by the same generated RFC 8927 validators that guard the network
boundary, and the same validators re-check anything loaded back out of
IndexedDB or session storage on a cold start — storage is just another
untrusted wire. State management stops being a framework category and
becomes three web platform primitives plus one pure function.

## The movement-shaped hole

Here is what struck me when I went looking for company: the pieces exist,
but the movement does not. What is out there, as of this writing:

- [Why htmx Does Not Have a Build Step](https://htmx.org/essays/no-build-step/)
  (Alexander Petros) — the best short case for build-step-free code:
  longevity, debuggability, and maintenance cost, argued from running an
  actual project.
- [The No-Build Manifesto](https://blakecrosley.com/blog/no-build-manifesto)
  (Blake Crosley) — production numbers from a real no-build site, honest
  about the tradeoffs.
- The vanilla-JS community — Chris Ferdinandi's long-running
  [vanilla JS podcast](https://vanillajspodcast.com/) and teaching, "for
  people who hate the complexity of modern front-end development".
- The classics the movement forgets it stands on: Rich Hickey's
  ["Simple Made Easy"](https://www.infoq.com/presentations/Simple-Made-Easy/),
  Dan McKinley's ["Choose Boring Technology"](https://mcfunley.com/choose-boring-technology),
  the [suckless philosophy](https://suckless.org/philosophy/), and
  ["Software Disenchantment"](https://tonsky.me/blog/disenchantment/).

Every one of these predates the LLM era or argues from aesthetics,
performance, or maintenance cost. None of them names the AI-era stakes:
that the model is the new framework, that the dependency graph is now a
hallucination surface, and that verification — not generation — is the
scarce resource. There is, as far as I can find, no shared manifesto for
**zero-dependency software in the age of abundant code generation**.

That is the gap this series is going to fill. Draft tenets, in raw form:

1. **The dependencies you don't have can't be wormed.** Supply-chain
   security is mostly a subtraction problem.
2. **Generate, don't import.** AOT-generate the code a contract requires
   from a standard, commit the output, review it like code — because it is.
3. **The model is the framework.** Prefer what the training corpus knows
   cold: platform primitives, vanilla language features, boring patterns.
4. **The build you don't have can't break.** Every transform between what
   you wrote and what runs is a trust boundary and a debugging tax. Keep
   both near zero.
5. **Validate at the boundary, then freeze.** Fail closed on the first bad
   datum; make verified data immutable so trust can't erode downstream.
6. **Same bytes for human, machine, and browser.** No transpiled drift
   between what the agent read, what you reviewed, and what executed.
7. **The empty result is a valid result.** Streams, not nulls; absence of
   data is data.
8. **Small enough to audit beats big enough to be convenient.** If it
   doesn't fit in one sitting, you are not reviewing it — you are hoping.
9. **Dependencies are a budget, not a ban.** Spend them where reinvention
   is genuinely dangerous (cryptography, concurrency primitives) and never
   for what the platform already ships.

## Steel-manning the other side

A position is only strong if it survives its objections:

- **"You'll reinvent a worse wheel."** Sometimes. The discipline answers
  with the budget rule: reinvention is for utilities, not for
  cryptography, concurrency, or codec-level correctness. And an
  AI-generated 200-line utility you can read entirely is a *better* wheel
  than a 400-dependency transitive graph nobody has read.
- **"The platform is a dependency too."** True — and it is the one
  dependency with a standards process, decades of backward compatibility,
  security response, and no install step. The browser *is* the framework;
  that is the point of building on web standards and testable web APIs.
- **"This doesn't scale to big teams."** Correct, and worth saying: the
  no-build crowd admits it. Vanilla-pod-js is scoped to what it can prove:
  browser-side consumption, small-to-medium surfaces, teams that can read
  their own glue. The skill says what it rejects on purpose.
- **"Zero dependencies is a luxury of small scope."** The counter-example
  is in this repository: a network-facing security proxy specified in RFC
  8927 schemas with a small, auditable Rust core. Scope discipline and
  dependency discipline are the same discipline applied at different
  layers.

## What this unlocks

The video did the movement a favour: it moved "zero dependencies" from
personality trait to named trend, with the receipts to make it credible.
What remains is to build the intellectual scaffolding — the why, the
limits, the practices — before the term gets flattened into a buzzword.
Planned follow-ups, each seeded from this document:

1. **"The model is the framework"** — a deeper dive on convention,
   training-data priors, and why boring code is the AI-native choice.
2. **"Generate, don't import"** — the RFC 8927 / jtd-codegen story as a
   general pattern: schemas as data, validators as build artifacts,
   verification as compilation.
3. **"The dependency budget"** — a practical decision framework for teams:
   what to never hand-roll, what to never install, and the review cost
   model that separates them.
4. **"Fail closed"** — the validation-and-freeze pattern as a general
   trust-boundary design, from NDJSON streams to credential custody.
5. **"The boundary equation"** — one schema, four artefacts, zero
   frameworks: the full-stack walk from database view to frozen browser
   row, using the functional-fetchers and Tripwire material as the
   worked example.
6. **"The architecture is the visible shape of the computation"** — full-
   arity kernels, lifecycle-ordered binding, and why a DI container is
   twenty lines of currying you will never have to debug.

The window is open now. The trend has a name, the incidents have case
studies, and the working software already exists. Somebody is going to
write the manifesto for this; it might as well be the people who were
building it before the trend had a name.

## Sources

- Fireship, ["The wild world of tech in 2027..."](https://youtu.be/sPGzM3ktJVE), The Code Report, 27 November 2026 (transcript quoted above).
- CISA, [Widespread Supply Chain Compromise Impacting npm Ecosystem](https://www.cisa.gov/news-events/alerts/2025/09/23/widespread-supply-chain-compromise-impacting-npm-ecosystem), 23 September 2025.
- Unit 42, ["Shai-Hulud" Worm Compromises npm Ecosystem](https://unit42.paloaltonetworks.com/npm-supply-chain-attack/); [The npm Threat Landscape](https://unit42.paloaltonetworks.com/monitoring-npm-supply-chain-attacks/).
- Datadog Security Labs, [The Shai-Hulud 2.0 npm worm](https://securitylabs.datadoghq.com/articles/shai-hulud-2.0-npm-worm/).
- Spracklen et al., [We Have a Package for You! A Comprehensive Analysis of Package Hallucinations by Code Generating LLMs](https://arxiv.org/abs/2504.08573), USENIX Security 2025.
- Alexander Petros, [Why htmx Does Not Have a Build Step](https://htmx.org/essays/no-build-step/).
- Blake Crosley, [The No-Build Manifesto](https://blakecrosley.com/blog/no-build-manifesto).
- Prompt Cult, [vanilla-pod-js skill](https://prompt-cult.github.io/prompt-cult/) and [jtd-codegen](https://github.com/prompt-cult/json-type-definition-RFC-8927).
- Prompt Cult, [late-arriver](https://github.com/prompt-cult/late-arriver) — durable-bus demo: BroadcastChannel + IndexedDB spool worker + late subscriber, gap-free and duplicate-free.
- simbo1905, gists: [vanilla-pod-js](https://gist.github.com/simbo1905/5be015475abd1a1611c0e996218ce4e7), [functional fetchers with vanilla JS, uv, JTD, and typed views](https://gist.github.com/simbo1905/e9ff1159497ddfaad195a224e8fb4a3d), [Tripwire API honest current pattern](https://gist.github.com/simbo1905/c5645488616d0374936659f798aeea68).
