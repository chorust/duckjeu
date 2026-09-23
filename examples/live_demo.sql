-- Live TypeSafe showcase. Uses only non-sensitive sample data.
-- Set DUCKJEU_API_KEY or TYPESAFE_API_KEY in the environment before running.

LOAD '@@EXTENSION@@';
SET duckjeu_provider = 'typesafe';

-- Three binary probability judgments.
SELECT id,
       jev_prob(message, 'the customer requests a refund') AS refund_probability
FROM read_parquet('examples/data/tickets.parquet')
WHERE id <= 3
ORDER BY id;

-- Three independent boolean judgments.
SELECT id,
       jev_bool(message, 'the customer requests a refund') AS refund_like
FROM read_parquet('examples/data/tickets.parquet')
WHERE id <= 3
ORDER BY id;

-- Three choice judgments over structured state.
SELECT price,
       jev_choice(
           struct_pack(price := price, load := load, wind := wind),
           'which market regime best describes this state?',
           ['normal', 'scarcity', 'oversupply']
       ) AS regime
FROM read_parquet('examples/data/market.parquet')
ORDER BY price
LIMIT 3;

-- Top-level NULL short-circuits without a provider request.
SELECT jev_prob(NULL, 'anything') AS null_state,
       jev_choice('state', 'question', NULL) AS null_choices;
