// Genre / tag classifier.
//
// Inputs come from two noisy sources: Open Library "subjects" (free-form library
// headings such as "Fantasy fiction" or "Description and travel") and Google Books
// "categories" (slash-delimited paths such as "Juvenile Fiction / Fantasy & Magic").
// Neither maps cleanly onto a shelf, so we score every genre from every signal and
// pick the winner, instead of trusting any single field.
//
// Scoring: for each signal (a subject, a category, the title, the description) and
// each genre, take the strongest matching rule, multiply by how much we trust that
// kind of signal, and sum across signals. A fiction/non-fiction check then damps the
// wrong side (so "Science fiction" never lands in Science). Dewey Decimal numbers,
// when present, add a coarse non-fiction signal.

/** Rule = [weight, subgenre|null, regex]. Regexes run against lowercase text. */
const R = (w, sub, re) => ({ w, sub, re });

// Order matters: it is the tiebreaker, and the order genres appear in the UI.
export const TAXONOMY = [
  // ---------- Fiction ----------
  { name: 'Science Fiction', kind: 'fiction', rules: [
    R(3, null, /science fiction|sci-?fi\b|speculative fiction/),
    R(2, 'Cyberpunk & Steampunk', /cyberpunk|steampunk/),
    R(2, 'Space Opera', /space opera|galactic empire|interstellar/),
    R(2, 'Dystopian', /dystopi|post-?apocalyptic|apocalyptic/),
    R(2, 'Time Travel', /time travel/),
    R(2, 'First Contact', /extraterrestrial|first contact|\baliens?\b/),
    R(1.5, null, /\bandroids?\b|interplanetary|space colon|artificial intelligence.*fiction|robots?.*fiction/),
  ] },
  { name: 'Fantasy', kind: 'fiction', rules: [
    R(3, null, /fantasy/),
    R(2, 'Epic Fantasy', /epic fantasy|high fantasy/),
    R(2, 'Urban Fantasy', /urban fantasy/),
    R(2, 'Dark Fantasy', /dark fantasy/),
    R(2, 'Fairy Tales & Myth', /fairy tales?|folk ?tales?|mythical|\bmyths?\b.*fiction/),
    R(1.5, 'Magic & Wizards', /\bmagic\b|wizards?|witch(es)?\b|sorcer|dragons?\b|\belves\b|\bmagicians?\b/),
  ] },
  { name: 'Mystery & Crime', kind: 'fiction', rules: [
    R(3, null, /mystery|mysteries|detective|whodunit|private investigators?|sleuth/),
    R(2, null, /crime (fiction|novels?|stories)|^crime$|murder|police procedural/),
    R(2, 'Detective', /detective|private investigators?|sleuth/),
    R(2, 'Cozy', /\bcozy\b|\bcosy\b/),
    R(2, 'Police Procedural', /police|procedural/),
    R(2, 'Noir & Hardboiled', /\bnoir\b|hard-?boiled/),
  ] },
  { name: 'Thriller & Suspense', kind: 'fiction', rules: [
    R(3, null, /thrillers?|suspense|techno-?thriller/),
    R(2.5, 'Espionage', /espionage|\bspy\b|\bspies\b|intelligence service/),
    R(2, 'Psychological', /psychological (thriller|suspense)/),
    R(2, 'Legal', /legal (thriller|stories)|lawyers.*fiction/),
    R(1.5, null, /conspirac|serial killers.*fiction|kidnapping|terroris.*fiction/),
  ] },
  { name: 'Romance', kind: 'fiction', rules: [
    R(3, null, /romance|romances|love stories/),
    R(2, 'Historical Romance', /historical romance|regency/),
    R(2, 'Contemporary Romance', /contemporary romance/),
    R(2, 'Paranormal Romance', /paranormal romance/),
    R(2, 'Romantic Suspense', /romantic suspense/),
    R(1.5, null, /courtship|man-woman relationships|love (affair|triangle)/),
  ] },
  { name: 'Horror', kind: 'fiction', rules: [
    R(3, null, /horror|ghost stories|haunted|\bgothic\b/),
    R(2, 'Supernatural', /supernatural|ghosts?\b|haunted|occult fiction|demons?\b/),
    R(2, 'Vampires & Werewolves', /vampires?|werewolves|werewolf/),
    R(2, 'Gothic', /\bgothic\b/),
    R(1.5, null, /zombies?|monsters?.*fiction|slasher/),
  ] },
  { name: 'Historical Fiction', kind: 'fiction', rules: [
    R(3, null, /historical (fiction|novels?)|^historical$/),
    R(2.5, null, /\b(history|war|revolution|empire|colonial|century|dynasty|medieval|world war|civil war)\b.*\bfiction$/),
    R(2.5, null, /\bfiction\b.*\bhistorical\b/),
  ] },
  { name: 'Classics', kind: 'either', rules: [
    R(3, null, /\bclassics?\b|classic (literature|fiction)|literary collections/),
    R(1.5, null, /victorian|(19th|nineteenth) century.*(fiction|literature)/),
  ] },
  { name: 'Literary Fiction', kind: 'fiction', rules: [
    R(3, null, /literary fiction|contemporary fiction|general fiction/),
    R(2, null, /psychological fiction|domestic fiction|bildungsroman|coming of age|family life.*fiction|satire/),
    R(1, null, /^fiction(, general)?$|^novels?$/),
  ] },
  { name: 'Short Stories & Anthologies', kind: 'either', rules: [
    R(3, null, /short stories|anthologies|anthology/),
    R(1.5, null, /collections$|\bessays\b/),
  ] },
  { name: 'Young Adult', kind: 'fiction', rules: [
    R(3, null, /young adult|\bya fiction\b|teen(age)? (fiction|readers)/),
    R(1.5, null, /\bteen(s|agers?)?\b|adolescen/),
  ] },
  { name: "Children's Books", kind: 'either', rules: [
    R(2.5, null, /juvenile|children'?s|\bfor children\b|\bages? \d/),
    R(2.5, 'Picture Books', /picture books?|board books?/),
    R(2, 'Early Readers & Chapter Books', /early readers?|chapter books?|readers?\b.*(grade|level)/),
    R(1.5, null, /bedtime|nursery rhymes?/),
  ] },
  { name: 'Graphic Novels & Comics', kind: 'either', rules: [
    R(3, null, /graphic novels?|\bcomics?\b|comic strips?|\bmanga\b|cartoons?|comic books?/),
    R(2, 'Manga', /\bmanga\b/),
    R(2, 'Superheroes', /superhero/),
  ] },
  { name: 'Poetry', kind: 'either', rules: [
    R(3, null, /poetry|\bpoems?\b|\bpoets?\b|\bverse\b|sonnets?/),
  ] },
  { name: 'Drama & Plays', kind: 'either', rules: [
    R(3, null, /^drama\b|\bplays\b|playwrights?|\btheat(er|re)\b|tragedies|comedies|shakespeare/),
  ] },
  { name: 'Humor', kind: 'either', rules: [
    R(3, null, /humou?r|\bfunny\b|\bjokes?\b|comic (novels|fiction)/),
    R(1.5, null, /\bsatire\b|\bcomedy\b/),
  ] },

  // ---------- Non-fiction ----------
  { name: 'Biography & Memoir', kind: 'nonfiction', rules: [
    R(3, null, /\bbiograph|autobiograph|memoirs?\b|personal narratives|diaries/),
    R(2, 'Memoir', /memoirs?\b|autobiograph|personal narratives/),
    R(2, 'Biography', /\bbiograph/),
    R(1.5, null, /\bletters\b|correspondence/),
  ] },
  { name: 'History', kind: 'nonfiction', rules: [
    R(3, null, /(?<!natural )(?<!oral )\bhistory\b|histories|archaeolog/),
    R(2, null, /world war|civil war|\bmedieval\b|renaissance|\bcolonial|\bempires?\b|ancient/),
    R(2, 'Ancient', /ancient|antiquity|\brome\b|\broman\b|\bgreece\b|\bgreek\b|egypt/),
    R(2, 'Medieval', /medieval|middle ages|crusades/),
    R(2, 'Military & War', /military|\bwars?\b|battles?|\barmy\b|\bnavy\b|world war|civil war/),
    R(2, 'United States', /united states|\bamerica(n)?\b/),
    R(2, 'Europe', /\beurope|england|france|germany|russia|britain|great britain|ireland|italy/),
    R(2, 'Asia', /\basia|china|japan|india\b|korea|vietnam/),
    R(2, 'Africa & Middle East', /africa|middle east|\bislamic world|arab/),
  ] },
  { name: 'Science & Nature', kind: 'nonfiction', rules: [
    R(3, null, /^science$|^sciences$|natural history|\b(physics|chemistry|biology|astronomy|geology|ecology|zoology|botany|neuroscience|cosmology|astrophysics|paleontology|genetics|evolution|oceanograph|meteorology)\b/),
    R(2, null, /\bmathematics|\bnature\b|\banimals\b|\bplants\b|scientists|\bspace\b.*exploration|universe/),
    R(2.5, 'Physics & Astronomy', /physics|astronom|cosmolog|astrophysics|universe|space exploration|quantum/),
    R(2.5, 'Biology & Evolution', /biolog|evolution|genetic|zoolog|botan|\bdna\b|neuroscience|microb/),
    R(2.5, 'Earth & Environment', /geolog|climate|environment|ecolog|oceans?\b|weather|conservation|global warming/),
    R(2.5, 'Mathematics', /mathemat|statistics|probabilit|geometry|algebra|calculus/),
    R(2, 'Nature & Animals', /\banimals\b|\bbirds\b|wildlife|\bnature\b|\bplants\b|insects|dinosaurs?|natural history/),
  ] },
  { name: 'Technology & Computing', kind: 'nonfiction', rules: [
    R(3, null, /comput(er|ing)|programming|software|algorithms?|data (structures|bases?|science)|machine learning|web (development|design)|cryptograph|cybersecurity|robotics/),
    R(2, null, /\btechnology\b|engineering|electronics|internet|artificial intelligence|\bhack/),
    R(2.5, 'Programming', /programming|software|algorithm|coding|python|javascript|\bjava\b|\bc\+\+|\bunix\b|\blinux\b/),
    R(2.5, 'AI & Data', /artificial intelligence|machine learning|data science|big data/),
    R(2.5, 'Engineering & Electronics', /engineering|electronics|mechanical|robotics/),
    R(2.5, 'Networks & Security', /network|security|cryptograph|internet|hacking|cyber/),
  ] },
  { name: 'Business & Economics', kind: 'nonfiction', rules: [
    R(3, null, /business|economics?\b|\bmanagement\b|marketing|finance|investing|investments?\b|entrepreneur|accounting|personal finance|\bmoney\b|\bsales\b|startups?/),
    R(1.5, null, /leadership|\bcareers?\b|negotiat|productivity/),
    R(2, 'Economics', /economics?\b|\bmarkets?\b|capitalism/),
    R(2, 'Finance & Investing', /finance|investing|investments?\b|\bmoney\b|stock market/),
    R(2, 'Management & Leadership', /management|leadership|organizational/),
    R(2, 'Marketing & Sales', /marketing|\bsales\b|advertising/),
    R(2, 'Entrepreneurship', /entrepreneur|startups?|small business/),
  ] },
  { name: 'Self-Help & Psychology', kind: 'nonfiction', rules: [
    R(3, null, /self-help|self help|self-improvement|personal (growth|development)|psycholog|conduct of life|self-actualization|mental health|mindfulness|motivation/),
    R(2, null, /\bhabits?\b|happiness|\bsuccess\b|emotions?\b|\bstress\b|anxiety|depression|cognitive|behavio(u)?r|interpersonal relations/),
    R(2.5, 'Psychology', /psycholog|cognitive|behavio(u)?r|neuropsych|mental health/),
    R(2.5, 'Self-Improvement', /self-help|self help|self-improvement|personal (growth|development)|\bhabits?\b|motivation|conduct of life|\bsuccess\b/),
    R(2.5, 'Mindfulness & Wellbeing', /mindfulness|happiness|\bstress\b|anxiety|emotions?\b/),
    R(2, 'Relationships', /interpersonal relations|\bmarriage\b|parenting|communication/),
  ] },
  { name: 'Philosophy', kind: 'nonfiction', rules: [
    R(3, null, /philosoph|metaphysics|epistemology|\bethics\b|stoic|existential|\blogic\b/),
    R(2, 'Ethics', /\bethics\b|\bmorality\b/),
    R(2, 'Stoicism', /stoic/),
    R(2, 'Existentialism', /existential/),
  ] },
  { name: 'Religion & Spirituality', kind: 'nonfiction', rules: [
    R(3, null, /religio|spiritual|\bbible\b|christian|islam|muslim|judaism|jewish|buddhis|hindu|theolog|prayer|mysticism|\bfaith\b|new age|meditation|\bgod\b/),
    R(2, 'Christianity', /christian|\bbible\b|church|theolog/),
    R(2, 'Islam', /islam|muslim|quran|koran/),
    R(2, 'Judaism', /judaism|jewish|torah/),
    R(2, 'Buddhism & Eastern', /buddhis|hindu|zen\b|taoism/),
    R(2, 'Spirituality', /spiritual|meditation|mysticism|new age/),
  ] },
  { name: 'Politics & Social Science', kind: 'nonfiction', rules: [
    R(3, null, /politic|government|democracy|social (science|problems|policy|conditions)|sociolog|anthropolog|international relations|public policy|civil rights|human rights|immigration|\bracism\b|feminis|communis|socialis/),
    R(1.5, null, /\bpoverty\b|\bgender\b|\bculture\b|\bclass\b|inequality|propaganda/),
    R(2.5, 'Politics & Government', /politic|government|democracy|public policy|election/),
    R(2.5, 'Sociology', /sociolog|social (problems|conditions|policy)|inequality|poverty/),
    R(2.5, 'Anthropology', /anthropolog|ethnolog|folklore/),
    R(2.5, 'Gender & Race', /feminis|\bgender\b|\brace\b|racism|civil rights/),
    R(2.5, 'International Relations', /international relations|foreign (policy|relations)|diplomacy/),
  ] },
  { name: 'Law & Legal', kind: 'nonfiction', rules: [
    R(3, null, /\blaw\b|\blegal\b|constitutional|jurisprudence|\bcourts?\b|criminal justice/),
  ] },
  { name: 'True Crime', kind: 'nonfiction', rules: [
    R(4, null, /true crime|serial killers|criminology|criminals?\b|murder.*(investigation|case|trial)/),
  ] },
  { name: 'Travel', kind: 'nonfiction', rules: [
    R(3, null, /\btravel|voyages|guidebooks?|tourism|description and travel|backpacking|expeditions?/),
    R(2, 'Guidebooks', /guidebooks?|\bguides?\b/),
    R(2, 'Travel Writing', /description and travel|narratives|\bjourneys?\b|voyages/),
  ] },
  { name: 'Cooking & Food', kind: 'nonfiction', rules: [
    R(3, null, /\bcook(ing|books?)?\b|recipes?|\bfood\b|cuisine|\bbaking\b|beverages?|\bwine\b|\bbeer\b|gastronom|vegetarian|vegan|cocktails?/),
    R(2, 'Baking', /\bbaking\b|bread|pastry|desserts?/),
    R(2, 'Drinks', /\bwine\b|\bbeer\b|cocktails?|coffee|\btea\b|beverages?|spirits/),
    R(2, 'Regional Cuisine', /cuisine|italian|french|asian|mexican|american cooking/),
    R(2, 'Vegetarian & Healthy', /vegetarian|vegan|gluten|healthy/),
  ] },
  { name: 'Health & Fitness', kind: 'nonfiction', rules: [
    R(3, null, /\bhealth\b|fitness|exercise|\bdiet(ing|s)?\b|medicine|medical|\bdiseases?\b|nutrition|\bsleep\b|wellness|\byoga\b|physiology|nursing/),
    R(2, 'Fitness & Exercise', /fitness|exercise|\byoga\b|running|weight (loss|training)|strength/),
    R(2, 'Diet & Nutrition', /\bdiet(ing|s)?\b|nutrition|weight loss/),
    R(2, 'Medicine', /medicine|medical|\bdiseases?\b|physiology|nursing|surgery/),
  ] },
  { name: 'Arts & Photography', kind: 'nonfiction', rules: [
    R(3, null, /\barts?\b|painting|drawing|photograph|architecture|sculpture|museums?|\bfashion\b|graphic design|illustration|art history|\bdesign\b/),
    R(2, 'Film & Television', /\bfilms?\b|cinema|motion pictures|television|screenwriting/),
    R(2, 'Architecture & Design', /architecture|\bdesign\b|interior/),
    R(2, 'Photography', /photograph/),
    R(2, 'Painting & Drawing', /painting|drawing|illustration|sculpture/),
  ] },
  { name: 'Music', kind: 'nonfiction', rules: [
    R(3, null, /\bmusic\b|musicians?|\bsongs?\b|\bjazz\b|\brock (music|and roll)|\bopera\b|composers?|\bguitar\b|\bpiano\b|orchestra|hip-?hop/),
  ] },
  { name: 'Home, Garden & Crafts', kind: 'nonfiction', rules: [
    R(3, null, /gardening|\bgardens?\b|home (improvement|repair)|\bcrafts?\b|woodwork|knitting|sewing|quilting|\bdiy\b|handicrafts|do-it-yourself|housekeeping|decluttering|\borganizing\b/),
    R(2, 'Gardening', /garden|horticultur|\bplants\b/),
    R(2, 'Crafts & DIY', /crafts?|woodwork|knitting|sewing|quilting|\bdiy\b|handicrafts|do-it-yourself/),
    R(2, 'Home & Organizing', /home (improvement|repair)|housekeeping|decluttering|organizing/),
    R(2, 'Pets', /\bpets?\b|\bdogs?\b|\bcats?\b/),
  ] },
  { name: 'Sports & Recreation', kind: 'nonfiction', rules: [
    R(3, null, /\bsports?\b|baseball|football|basketball|soccer|\bgolf\b|tennis|hockey|olympic|athletes?|cycling|climbing|hiking|outdoors?\b|fishing|hunting|camping|\bchess\b|poker|recreation|martial arts|boxing/),
    R(2, 'Outdoors', /hiking|camping|fishing|hunting|climbing|outdoors?\b/),
    R(2, 'Team Sports', /baseball|football|basketball|soccer|hockey|team sports/),
    R(2, 'Games & Puzzles', /\bgames?\b|\bchess\b|puzzles?|poker|video games/),
  ] },
  { name: 'Language & Linguistics', kind: 'nonfiction', rules: [
    R(3, null, /\blanguages?\b|linguistic|grammar|etymology|phrasebook|vocabulary|translation/),
  ] },
  { name: 'Education & Reference', kind: 'nonfiction', rules: [
    R(3, null, /\breference\b|dictionar(y|ies)|encyclopedi|almanacs?|handbooks?|study guides?|\beducation\b|\bteaching\b|textbooks?|test prep|library science|quotations/),
    R(2, 'Dictionaries & Encyclopedias', /dictionar|encyclopedi|almanacs?|quotations/),
    R(2, 'Study & Test Prep', /study guides?|test prep|textbooks?|examinations/),
    R(2, 'Writing & Publishing', /\bwriting\b|authorship|publishing|journalism|composition|rhetoric/),
    R(2, 'Teaching', /\bteaching\b|\beducation\b|teachers?|pedagog/),
  ] },
];

const GENRE_BY_NAME = new Map(TAXONOMY.map((g) => [g.name, g]));
export const GENRE_NAMES = TAXONOMY.map((g) => g.name);
export const UNCATEGORIZED = 'Uncategorized';

// How much we trust each kind of signal.
const TRUST = { subject: 1, category: 1.2, dewey: 1.2, title: 0.6, description: 0.15 };
const MIN_SCORE = 1;           // below this we call it Uncategorized
const SECONDARY_RATIO = 0.45;  // other genres >= 45% of the winner become tags
const FICTION_DAMP = 0.35;     // multiplier applied to the "wrong side" genres

// ---------------------------------------------------------------------------

export function normalizeSignal(s) {
  return String(s ?? '').toLowerCase().replace(/&amp;/g, '&').replace(/\s+/g, ' ').trim();
}

/** The full string plus its path segments ("a -- b / c" -> [whole, a, b, c]). */
function variants(text) {
  const parts = text.split(/\s*(?:--|\/|;)\s*/).filter(Boolean);
  return parts.length > 1 ? [text, ...parts] : [text];
}

const FICTION_RE = /\bfiction\b|\bnovels?\b|\bstories\b|\bfairy tales?\b/;
const NONFICTION_RE = /non-?fiction|\bbiograph|autobiograph|\bmemoirs?\b/;

/** Dewey Decimal -> [genre, sub, weight, isFiction]. Coarse by design. */
export function deweyHints(code) {
  const raw = String(code ?? '').trim().toLowerCase();
  if (!raw) return null;
  if (/^\[?(fic|e)\]?$/.test(raw)) return { fiction: true };
  if (/^\[?b\]?$/.test(raw) || /^\[?92\]?$/.test(raw)) return { genre: 'Biography & Memoir', sub: 'Biography' };
  const m = raw.match(/^\[?(\d{1,3})/);
  if (!m) return null;
  const n = parseInt(m[1].padEnd(3, '0'), 10);
  const g = (genre, sub = null) => ({ genre, sub });
  if (n >= 800 && n < 900) {
    const dec = Math.floor((n % 100) / 10), unit = n % 10;
    if (n >= 810 && n <= 899) {
      if (unit === 1) return g('Poetry');
      if (unit === 2) return g('Drama & Plays');
      // 813/823/... only says "fiction in language X": it flags fiction but must not vote for a genre,
      // or every genre novel with a Dewey number would be pulled towards Literary Fiction.
      if (unit === 3) return { fiction: true };
      if (unit === 7) return g('Humor');
      if (unit === 4 || unit === 8) return g('Short Stories & Anthologies');
    }
    void dec;
    return null;
  }
  if (n >= 4 && n <= 6) return g('Technology & Computing', n === 5 ? 'Programming' : null);
  if (n < 100) return g('Education & Reference');
  if (n < 150) return g('Philosophy');
  if (n < 160) return g('Self-Help & Psychology', 'Psychology');
  if (n < 200) return g('Philosophy');
  if (n < 300) return g('Religion & Spirituality');
  if (n < 320) return g('Politics & Social Science', 'Sociology');
  if (n < 330) return g('Politics & Social Science', 'Politics & Government');
  if (n < 340) return g('Business & Economics', 'Economics');
  if (n < 350) return g('Law & Legal');
  if (n < 370) return g('Politics & Social Science');
  if (n < 380) return g('Education & Reference', 'Teaching');
  if (n < 400) return g('Politics & Social Science');
  if (n < 500) return g('Language & Linguistics');
  if (n < 520) return g('Science & Nature', 'Mathematics');
  if (n < 530) return g('Science & Nature', 'Physics & Astronomy');
  if (n < 600) return g('Science & Nature');
  if (n < 610) return g('Technology & Computing');
  if (n < 620) return g('Health & Fitness', 'Medicine');
  if (n < 630) return g('Technology & Computing', 'Engineering & Electronics');
  if (n < 640) return g('Home, Garden & Crafts', 'Gardening');
  if (n < 650) return n >= 641 && n < 642 ? g('Cooking & Food') : g('Home, Garden & Crafts');
  if (n < 660) return g('Business & Economics');
  if (n < 700) return g('Technology & Computing');
  if (n < 780) return g('Arts & Photography');
  if (n < 790) return g('Music');
  if (n < 800) return n >= 791 && n < 792 ? g('Arts & Photography', 'Film & Television') : g('Sports & Recreation');
  if (n < 910) return g('History');
  if (n < 920) return g('Travel');
  if (n < 930) return g('Biography & Memoir', 'Biography');
  return g('History');
}

/**
 * Classify a book.
 * @param {{title?:string, subtitle?:string, subjects?:string[], categories?:string[], dewey?:string[], description?:string}} input
 * @returns {{genre:string, subgenre:string, secondary:string[], tags:string[], confidence:'high'|'medium'|'low', needsReview:boolean, scores:Record<string,number>}}
 */
export function classify(input) {
  const subjects = (input.subjects ?? []).map(normalizeSignal).filter(Boolean);
  const categories = (input.categories ?? []).map(normalizeSignal).filter(Boolean);
  const title = normalizeSignal([input.title, input.subtitle].filter(Boolean).join(' '));
  const description = normalizeSignal(input.description).slice(0, 2000);

  const signals = [
    ...subjects.map((text) => ({ text, trust: TRUST.subject, structured: true })),
    ...categories.map((text) => ({ text, trust: TRUST.category, structured: true })),
    ...(title ? [{ text: title, trust: TRUST.title }] : []),
    ...(description ? [{ text: description, trust: TRUST.description }] : []),
  ];

  let fictionFlag = false;
  let nonfictionFlag = false;
  for (const s of signals) {
    if (!s.structured) continue;
    for (const v of variants(s.text)) {
      if (NONFICTION_RE.test(v)) nonfictionFlag = true;
      else if (FICTION_RE.test(v)) fictionFlag = true;
    }
  }

  const score = new Map();                 // genre -> total
  const subScore = new Map();              // genre -> Map(sub -> total)
  const bump = (m, k, v) => m.set(k, (m.get(k) ?? 0) + v);

  for (const s of signals) {
    const vs = variants(s.text);
    for (const g of TAXONOMY) {
      let best = 0;
      const bestSub = new Map();
      for (const rule of g.rules) {
        if (!vs.some((v) => rule.re.test(v))) continue;
        if (rule.w > best) best = rule.w;
        if (rule.sub && rule.w > (bestSub.get(rule.sub) ?? 0)) bestSub.set(rule.sub, rule.w);
      }
      if (best > 0) {
        bump(score, g.name, best * s.trust);
        if (bestSub.size) {
          if (!subScore.has(g.name)) subScore.set(g.name, new Map());
          for (const [sub, w] of bestSub) bump(subScore.get(g.name), sub, w * s.trust);
        }
      }
    }
  }

  for (const code of input.dewey ?? []) {
    const h = deweyHints(code);
    if (!h) continue;
    if (h.fiction) fictionFlag = true;
    if (h.genre) {
      bump(score, h.genre, 2 * TRUST.dewey);
      if (h.sub) {
        if (!subScore.has(h.genre)) subScore.set(h.genre, new Map());
        bump(subScore.get(h.genre), h.sub, 2 * TRUST.dewey);
      }
    }
  }

  // Fiction vs non-fiction: damp the side the evidence argues against.
  for (const [name, val] of score) {
    const kind = GENRE_BY_NAME.get(name).kind;
    if (fictionFlag && !nonfictionFlag && kind === 'nonfiction') score.set(name, val * FICTION_DAMP);
    if (nonfictionFlag && !fictionFlag && kind === 'fiction') score.set(name, val * FICTION_DAMP);
  }

  const order = new Map(GENRE_NAMES.map((n, i) => [n, i]));
  const ranked = [...score.entries()]
    .filter(([, v]) => v >= MIN_SCORE)
    .sort((a, b) => b[1] - a[1] || order.get(a[0]) - order.get(b[0]));

  let genre = UNCATEGORIZED;
  let subgenre = '';
  let confidence = 'low';
  let secondary = [];

  if (ranked.length) {
    genre = ranked[0][0];
    const top = ranked[0][1];
    const runnerUp = ranked[1]?.[1] ?? 0;
    confidence = top >= 3 && top >= runnerUp * 1.5 ? 'high' : top >= 2 ? 'medium' : 'low';
    secondary = ranked
      .slice(1)
      .filter(([, v]) => v >= Math.max(1.5, top * SECONDARY_RATIO))
      .map(([n]) => n)
      .slice(0, 3);
    const subs = subScore.get(genre);
    if (subs?.size) subgenre = [...subs.entries()].sort((a, b) => b[1] - a[1])[0][0];
  } else if (fictionFlag) {
    genre = 'Literary Fiction';
    confidence = 'low';
  }

  const tags = buildTags({ subjects: input.subjects ?? [], categories: input.categories ?? [], genre, subgenre, secondary });

  return {
    genre,
    subgenre,
    secondary,
    tags,
    confidence,
    needsReview: genre === UNCATEGORIZED || confidence === 'low',
    scores: Object.fromEntries(ranked.map(([n, v]) => [n, Math.round(v * 100) / 100])),
  };
}

// ---------------------------------------------------------------------------
// Tags

const JUNK_TAG = /^nyt:|^(reading level|lexile|accelerated reader|ar )|large (print|type)|daisy|accessible book|in library|internet archive|overdrive|open library|^new york times|^\d{4}$|^(fiction|general|nonfiction|non-fiction|juvenile fiction|juvenile nonfiction|criticism and interpretation|specimens|translations into english|history and criticism|miscellanea|collections)$/i;

/** Clean raw subject/category strings into short, deduplicated, display-ready tags. */
export function buildTags({ subjects = [], categories = [], genre = '', subgenre = '', secondary = [] }) {
  const counts = new Map();
  const add = (raw, boost = 0) => {
    const t = String(raw)
      .replace(/\([^)]*\)/g, ' ')          // "(Fictitious character)"
      .replace(/&amp;/g, '&')
      .replace(/\s+/g, ' ')
      .replace(/[.,;:]+$/g, '')
      .trim()
      .toLowerCase();
    if (t.length < 3 || t.length > 40 || JUNK_TAG.test(t)) return;
    counts.set(t, (counts.get(t) ?? 0) + 1 + boost);
  };
  for (const raw of [...subjects, ...categories]) {
    for (const part of String(raw).split(/\s*(?:--|\/|;)\s*/)) add(part);
  }
  if (subgenre) add(subgenre, 5);
  for (const g of secondary) add(g, 4);

  const primary = String(genre).toLowerCase();
  return [...counts.entries()]
    .filter(([t]) => t !== primary)
    .sort((a, b) => b[1] - a[1])
    .map(([t]) => t)
    .slice(0, 12);
}
