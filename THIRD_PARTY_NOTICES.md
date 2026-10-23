# Third-party notices

## bloub avatar engine

The SVG animation engine in `apps/desktop/src/presence/engine/` is ported
from [bloub](https://github.com/jeremy-prt/bloub) by Jérémy Perret.

Copyright (c) 2026 Jérémy Perret. Licensed under the MIT License. The full
notice and permission text are retained in
[`LICENSE-bloub`](apps/desktop/src/presence/engine/LICENSE-bloub), and each
ported source file carries the copyright and MIT attribution.

Pegoles adapts the TypeScript port with English identifiers and comments.
The React renderer and controller use Pegoles' silver brand tokens and
existing Core presence signals. The upstream geometry, timings, expression
profiles, blink schedule and gaze calculations are retained. No upstream
application services or runtime dependencies are included.
