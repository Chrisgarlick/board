# Site infrastructure: base stacks to build on

Notes from 11 November 2026, to come back to. The aim is to be able to take a prospect off
Wix, Squarespace or similar and onto something better, fast, without maintaining more stacks
than one person can keep safe.

## What's already there

- **Kit** (`~/dev/kit`): the component library for Go, Laravel, WordPress or plain HTML, with
  one set of design tokens. Early: tokens, core runtime, first components.
- **cg-cms** (`~/dev/cg-cms`): the Laravel CMS, in production on chrisgarlick.com. Pages
  rendered to disk and served by nginx. Not built yet: roles and 2FA.
- **cms** (`~/dev/cms`): the Go CMS. A single binary, one instance per site, starters (brochure,
  blog, blank), mandatory 2FA, blue/green deploys with rollback.
- **wp-vite-plugin** (`~/dev/wp-vite-plugin`): Vite for WordPress themes.
- The pipeline around them: Scout (prospects, audits, brand), Hub (client knowledge), Design,
  Board, Typeset, Motion, Kritano (audits and monitoring).

## The principle: fewer stacks, not more

Every base theme has to be kept updated, secure and compatible for every client still on it.
Six bases (WordPress, WordPress with WooCommerce, cg-cms, cg-cms with e-commerce, Shopify, the
Go cms) is a maintenance job before anything is sold.

What we're up against: Wix and Squarespace cost these businesses about £15 to £30 a month, and
they can edit the site themselves. To move them, beat that on speed, results and support, and
keep them able to edit their own content.

## Recommendation

1. **One main stack for brochure sites** (most prospects). Our own CMS: cg-cms or the Go `cms`,
   not both. Choose the one whose admin a non-technical owner finds easiest. Leaning towards
   the Go `cms`: starters already, one instance per client, safe deploys, so many clients can
   sit on one droplet with a hosting and care fee. That monthly fee replaces their Wix bill.

2. **No e-commerce in our own CMS.** Payments, VAT, stock, shipping, refunds and card security
   become our problem. For shops:
   - **Shopify** is the right answer for most small shops (many already use Square or Shopify
     POS). No local development needed to start: a good theme and the theme editor. Shopify
     CLI later, when a client justifies it.
   - **WooCommerce** for clients already on WordPress, or who need it for other reasons.

3. **WordPress: one base theme, not two.** Built on Kit's tokens with wp-vite-plugin, with
   WooCommerce support switched on when the plugin is active, not a separate shop theme. A
   small, vetted plugin set, by job: backups, security, caching, image optimisation, forms,
   SEO, spam. As few as possible: every plugin is something that falls out of date (Scout now
   flags exactly that). WordPress for clients who need its ecosystem (memberships, complex
   bookings) and for rescuing sites already on it.

## What matters more than more themes

1. **A Wix and Squarespace migration pipeline.** Scout already reads a site's copy, brand
   (colours, fonts, logo) and photos. Turn that into an importer that brings their current site
   into a CMS starter, so moving off Wix is a day's work, not a week's.
2. **A section library in Kit**, built once for every stack: hero, services, reviews, gallery,
   call to action, FAQ, contact and map, booking.
3. **Bookings and payments by integration, not building**: Square Bookings, Cal.com, Stripe
   payment links. Trades, salons and coaches mostly need "book" or "get a quote".
4. **A care plan as the product**: hosting, updates, backups, uptime and audit monitoring
   (Kritano), small monthly changes. What we sell against their Wix bill.
5. **A go-live checklist and templates**: moving the domain and DNS (what scares owners), email
   (Google Workspace or Microsoft 365), redirects from old URLs so they keep their Google
   rankings, Google Business Profile, analytics, cookie banner, privacy and terms pages.
6. **One command to start a client**: Hub space, then a repo from the starter, then deployment
   on the droplet.

## Order

1. Choose the brochure stack (compare the Go `cms` and cg-cms admins side by side) and polish
   its starter with Kit sections.
2. The Wix and Squarespace importer, built on what Scout already does.
3. The care plan and go-live checklist.
4. The WordPress base theme (with WooCommerce support) and the plugin set.
5. Shopify, only when a client needs it.




## Chris Comments
1. go-cms is not being used for clients, that Kritano only. I will keep my clients as PHP based, so laravel or wordpress. If needed, shopify. But i'd rather migrate users from shopify to either using cg-cms as eccomerce once built OR wordpress woocommerce. 
2. Yes scout should have an export functionality. It will need to be fully planned though, because content will need to come, somehow the images will need to be moved over too. It can also only happen on published content, so anything in drafts or bin I can't bring over... 
3. Yes a section set of components would be useful. I think we can use design to build these out so that we have visual representations of each. 
4. Yes care plan is 100% required, but think this is something that will come from talking to people and seeing what they need. Right now, it would be maintenance (backups), potentially using ~/dev/sentry on their site for security once banning is implemented, uptime monitoring, auditing etc. These would be by default. 
5. go live checklist is also dependent on the client, so right now can be left. 
6. dunno if we need to go that far as yet. 
7. I want to have the infrastructure down for wordpress, cg-cms, potentially shopify, so that I can just have base themes that I can style dependent on the user. So i think this will be blocked by the kit section components? 
## Revised plan (after Chris's comments)

Client sites are PHP: cg-cms (Laravel) or WordPress, Shopify only if needed. The Go `cms` stays
Kritano's. Go-live checklist and one-command setup wait.

1. **Section designs in Design**: hero, services, reviews, gallery, call to action, FAQ,
   contact and map, booking, product grid, product page. Visual first, from the Hub library
   styles.
2. **Sections built in Kit**: HTML and CSS on the tokens, as Blade and WordPress PHP partials.
3. **Alongside 1 and 2:**
   - WordPress base theme skeleton: Vite (wp-vite-plugin), tokens, templates, WooCommerce
     templates when the plugin is active, and the plugin set (backups, security, caching,
     images, forms, SEO, spam).
   - cg-cms starter theme, and roles and 2FA before any client logs in.
4. **Scout export and migration, planned first**: published content only; Squarespace's XML
   export or reading the live site (Wix has no export); original-size images downloaded and
   re-uploaded; a redirect map from old URLs; WordPress import file or a cg-cms import command.
5. **cg-cms e-commerce, specified first**: Stripe Checkout (card details never touch our
   servers), Stripe Tax for VAT, orders by webhook, simple stock and flat shipping to start.
   Then moving shops off Shopify becomes possible.
6. **Care plan defaults**: backups, Sentry (once banning is in), uptime monitoring, Kritano
   audits, and a monthly care report through Typeset from Scout's health checks (SSL and
   domain expiry, out-of-date plugins, PHP support).
7. **Shopify base theme**, last: Liquid sections using Kit's tokens and styles.
