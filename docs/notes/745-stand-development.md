# §745. Stand development keeps the negative's contrast (2026-11-28)

In 0.5.0, Semi-stand and Stand made a flat grey picture on the eight
films with fitted development curves (§740): Tri-X 400, T-Max 100 and
400, HP5 Plus, FP4 Plus, and Delta 100, 400 and 3200. On Tri-X, the
picture's 1st to 99th percentiles went from 31–240 at 30 s agitation to
122–140 at stand. Two causes.

**The time.** The development time came from a rate that was linear in
film-darkroom's agitation index, with the slope fitted between two
points: Kodak's large tank (agitation every minute, index 0.67) and its
small tank (30 s, index 0.80), about 1.2. Carried down to stand (index
0) it gave a fifth of the normal time. A printer does not stand a film
for its tank time; the stand chart gives a time of its own. Now the
normal time, Development 0, is the time for the agitation chosen, and
agitation does not change it. Agitation still sets the compensation
and the edge effect. The tank-rate slope (`small_tank_rate`) is gone.

**The compensation.** Stand holds back the highlights: the developer
exhausts where the film develops the most silver. The model slowed
development by the fitted family's limit density, `d_inf`. On Delta 400
and T-Max 400 the fit's limit lies far above any print, so the slowing
reached the midtones and the shadows. Now it follows the density the
film reaches without compensation.

At stand, a fitted film keeps 59 % (Delta 400 in Rodinal 1+50) to 95 %
(Delta 3200 in D-76) of its midtone slope at 30 s; Rodinal compensates
the most and D-76 the least. A test holds every fitted film, in every
developer, at semi-stand and stand, over 55 %.

An edit at 30 s agitation, the default, develops as before. Every
minute and continuous agitation now develop for the same time as 30 s:
every minute prints with a little more contrast than in 0.5.0, and
continuous with a little less.
