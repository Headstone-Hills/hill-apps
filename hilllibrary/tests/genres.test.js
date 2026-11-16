import test from 'node:test';
import assert from 'node:assert/strict';
import { classify, buildTags, deweyHints, GENRE_NAMES, TAXONOMY } from '../server/genres.js';

// Subjects/categories modelled on how these books are typically catalogued by
// Open Library (subjects) and Google Books (categories).
const FIXTURES = [
  { name: 'Harry Potter', expect: 'Fantasy', input: {
    title: "Harry Potter and the Philosopher's Stone",
    subjects: ['Fantasy fiction', 'Wizards', 'Magic', 'Boarding schools', 'Juvenile fiction', 'England', 'Fiction'],
    categories: ['Juvenile Fiction / Fantasy & Magic'] } },
  { name: 'Dune', expect: 'Science Fiction', input: {
    title: 'Dune', subjects: ['Science fiction', 'Fiction', 'Ecology', 'Deserts', 'Politics and government'],
    categories: ['Fiction / Science Fiction / General'] } },
  { name: 'Gone Girl', expect: 'Thriller & Suspense', input: {
    title: 'Gone Girl', subjects: ['Fiction', 'Thrillers', 'Marriage', 'Missing persons', 'Psychological fiction'],
    categories: ['Fiction / Thrillers / Suspense'] } },
  { name: 'Murder on the Orient Express', expect: 'Mystery & Crime', input: {
    title: 'Murder on the Orient Express', subjects: ['Detective and mystery stories', 'Poirot, Hercule (Fictitious character)', 'Fiction'],
    categories: ['Fiction / Mystery & Detective / General'] } },
  { name: 'The Guns of August', expect: 'History', input: {
    title: 'The Guns of August', subjects: ['World War, 1914-1918', 'History', 'Diplomatic history', 'Europe'],
    categories: ['History / Military / World War I'] } },
  { name: 'Cosmos', expect: 'Science & Nature', input: {
    title: 'Cosmos', subjects: ['Astronomy', 'Cosmology', 'Universe', 'Science'],
    categories: ['Science / Astronomy'], dewey: ['520'] } },
  { name: 'Clean Code', expect: 'Technology & Computing', input: {
    title: 'Clean Code', subjects: ['Computer programming', 'Software engineering', 'Agile software development'],
    categories: ['Computers / Software Development & Engineering / General'], dewey: ['005.1'] } },
  { name: 'Educated', expect: 'Biography & Memoir', input: {
    title: 'Educated', subtitle: 'A Memoir', subjects: ['Memoir', 'Mormons', 'Education', 'Family'],
    categories: ['Biography & Autobiography / Personal Memoirs'] } },
  { name: 'Thinking, Fast and Slow', expect: 'Self-Help & Psychology', input: {
    title: 'Thinking, Fast and Slow', subjects: ['Thought and thinking', 'Decision making', 'Cognitive psychology', 'Intuition'],
    categories: ['Psychology / Cognitive Psychology & Cognition'] } },
  { name: 'Atomic Habits', expect: 'Self-Help & Psychology', input: {
    title: 'Atomic Habits', subjects: ['Habit', 'Self-help', 'Conduct of life', 'Success'],
    categories: ['Self-Help / Personal Growth / Success'] } },
  { name: 'Joy of Cooking', expect: 'Cooking & Food', input: {
    title: 'Joy of Cooking', subjects: ['Cooking, American', 'Cookbooks'], categories: ['Cooking / General'], dewey: ['641.5973'] } },
  { name: 'Maus', expect: 'Graphic Novels & Comics', input: {
    title: 'Maus', subjects: ['Graphic novels', 'Holocaust, Jewish (1939-1945)', 'Comic books, strips, etc'],
    categories: ['Comics & Graphic Novels / Literary'] } },
  { name: 'Pride and Prejudice', expectAny: ['Classics', 'Romance'], input: {
    title: 'Pride and Prejudice', subjects: ['Love stories', 'Courtship', 'Social life and customs', 'Classic Literature', 'England -- Fiction'],
    categories: ['Fiction / Classics'] } },
  { name: 'Sapiens-like (anthropology/history)', expectAny: ['History', 'Politics & Social Science', 'Science & Nature'], input: {
    title: 'Sapiens', subjects: ['Human beings', 'Civilization', 'Anthropology', 'Evolution', 'History'],
    categories: ['History / World / General'] } },
  { name: 'Historical novel', expect: 'Historical Fiction', input: {
    title: 'Wolf Hall', subjects: ['Historical fiction', 'Cromwell, Thomas, 1485?-1540 -- Fiction', 'Great Britain -- History -- Henry VIII, 1509-1547 -- Fiction'],
    categories: ['Fiction / Historical / General'] } },
  { name: 'Romance novel', expect: 'Romance', input: {
    title: 'The Hating Game', subjects: ['Romance fiction', 'Contemporary romance', 'Fiction'], categories: ['Fiction / Romance / Contemporary'] } },
  { name: 'Horror', expect: 'Horror', input: {
    title: 'The Shining', subjects: ['Horror fiction', 'Haunted hotels', 'Ghost stories', 'Fiction'], categories: ['Fiction / Horror'] } },
  { name: 'Travel', expect: 'Travel', input: {
    title: 'A Walk in the Woods', subjects: ['Description and travel', 'Appalachian Trail', 'Hiking'], categories: ['Travel / United States / General'] } },
  { name: 'Poetry', expect: 'Poetry', input: {
    title: 'Milk and Honey', subjects: ['Poetry', 'American poetry'], categories: ['Poetry / American / General'] } },
  { name: "Children's picture book", expect: "Children's Books", input: {
    title: 'Goodnight Moon', subjects: ['Picture books', 'Bedtime', 'Juvenile fiction'], categories: ['Juvenile Fiction / Bedtime & Dreams'] } },
];

for (const f of FIXTURES) {
  test(`classify: ${f.name}`, () => {
    const r = classify(f.input);
    if (f.expect) assert.equal(r.genre, f.expect, JSON.stringify(r.scores));
    else assert.ok(f.expectAny.includes(r.genre), `${r.genre} not in ${f.expectAny} ${JSON.stringify(r.scores)}`);
  });
}

test('"science fiction" never lands in Science & Nature', () => {
  const r = classify({ title: 'X', subjects: ['Science fiction'], categories: ['Fiction / Science Fiction / General'] });
  assert.equal(r.genre, 'Science Fiction');
  assert.ok(!(r.scores['Science & Nature'] > r.scores['Science Fiction']));
});

test('subgenre is chosen within the winning genre', () => {
  const r = classify({ title: 'Neuromancer', subjects: ['Science fiction', 'Cyberpunk', 'Fiction'], categories: [] });
  assert.equal(r.genre, 'Science Fiction');
  assert.equal(r.subgenre, 'Cyberpunk & Steampunk');
});

test('fiction with no genre evidence falls back to Literary Fiction, flagged for review', () => {
  const r = classify({ title: 'Untitled', subjects: ['Fiction'], categories: [] });
  assert.equal(r.genre, 'Literary Fiction');
  assert.equal(r.needsReview, true);
});

test('no evidence at all -> Uncategorized + needs review', () => {
  const r = classify({ title: 'Zzyzx', subjects: [], categories: [] });
  assert.equal(r.genre, 'Uncategorized');
  assert.equal(r.needsReview, true);
});

test('clear cases are high confidence and not flagged', () => {
  const r = classify({ title: 'Dune', subjects: ['Science fiction', 'Fiction'], categories: ['Fiction / Science Fiction / General'] });
  assert.equal(r.confidence, 'high');
  assert.equal(r.needsReview, false);
});

test('secondary genres become tags; primary genre does not', () => {
  const r = classify(FIXTURES[0].input);
  assert.ok(r.secondary.includes("Children's Books"), JSON.stringify(r.scores));
  assert.ok(r.tags.includes("children's books"));
  assert.ok(!r.tags.includes('fantasy'));
});

test('Dewey hints', () => {
  assert.equal(deweyHints('641.5973').genre, 'Cooking & Food');
  assert.equal(deweyHints('005.133').genre, 'Technology & Computing');
  assert.equal(deweyHints('523.1').genre, 'Science & Nature');
  assert.equal(deweyHints('813.54').fiction, true);
  assert.equal(deweyHints('813.54').genre, undefined); // fiction flag only, no genre vote
  assert.equal(deweyHints('[Fic]').fiction, true);
  assert.equal(deweyHints('92').genre, 'Biography & Memoir');
  assert.equal(deweyHints(''), null);
});

test('buildTags drops catalogue junk, parentheticals, years; dedupes; caps', () => {
  const tags = buildTags({
    subjects: ['Large type books', 'nyt:hardcover-fiction=2019-01-01', 'Accessible book', 'Protected DAISY',
      'Harry Potter (Fictitious character)', 'Wizards', 'Wizards', '2019', 'In library', 'Fiction -- General'],
    categories: ['Juvenile Fiction / Fantasy & Magic'],
  });
  assert.ok(tags.includes('harry potter'));
  assert.ok(tags.includes('wizards'));
  assert.ok(tags.includes('fantasy & magic'));
  for (const bad of ['large type books', 'accessible book', 'protected daisy', 'in library', '2019', 'fiction', 'general'])
    assert.ok(!tags.includes(bad), bad);
  assert.equal(new Set(tags).size, tags.length);
  assert.ok(buildTags({ subjects: Array.from({ length: 50 }, (_, i) => `topic number ${i}`) }).length <= 12);
});

test('taxonomy sanity: unique names, every sub belongs to rules, regexes are not global', () => {
  assert.equal(new Set(GENRE_NAMES).size, GENRE_NAMES.length);
  for (const g of TAXONOMY) for (const r of g.rules) assert.ok(!r.re.global && !r.re.sticky, g.name);
});

test('generic "General" path segment does not leak fiction score into non-fiction', () => {
  const r = classify({ title: 'Sapiens', subjects: ['History'], categories: ['History / World / General'] });
  assert.equal(r.scores['Literary Fiction'], undefined);
});

test('regression: a generic fiction Dewey number (813) must not outvote a specific genre', () => {
  for (const [dewey, subjects, cats, want] of [
    [['813/.54'], ['Science fiction', 'Fiction', 'Ecology'], [], 'Science Fiction'],
    [['823.912'], ['Fantasy fiction', 'Fiction', 'Dragons'], [], 'Fantasy'],
    [['813.6'], ['Detective and mystery stories', 'Fiction'], [], 'Mystery & Crime'],
    [['813/.54'], ['Fiction'], [], 'Literary Fiction'], // nothing more specific -> literary fallback still works
  ]) {
    assert.equal(classify({ title: 't', subjects, categories: cats, dewey }).genre, want, subjects.join());
  }
});
