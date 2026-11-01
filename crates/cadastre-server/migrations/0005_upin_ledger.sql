-- UPIN ledger: every UPIN ever issued, so none is ever issued twice.
--
-- A UPIN is the key the Aumentum Registry (LRS43 on 10.10.10.5) uses to link a registry property
-- to a cadastral parcel, and once a registry transaction is Completed that link can no longer be
-- changed. Reusing a UPIN would silently point a completed title at a different parcel. Before
-- this migration the allocator started at 1 and ignored imported Aumentum UPINs.
--
-- The ledger records UPINs from every source and every status (active, pending, historic,
-- archived, import exceptions, and new postings). Rows are never deleted, even when the parcel
-- itself is. The allocator is always kept above the highest UPIN in the ledger.

CREATE TABLE IF NOT EXISTS cadastre.issued_upin (
    upin       text PRIMARY KEY,
    source     text NOT NULL,          -- 'posted' | 'import:<run>:<status>' | 'import-exception:<run>' | 'existing'
    first_seen timestamptz NOT NULL DEFAULT now()
);

-- Protect the ledger from deletion and from rewriting a UPIN (an append-only record).
CREATE OR REPLACE FUNCTION cadastre.issued_upin_append_only() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'cadastre.issued_upin is append-only: UPIN % cannot be %', OLD.upin, lower(TG_OP);
END $$;

DROP TRIGGER IF EXISTS issued_upin_append_only ON cadastre.issued_upin;
CREATE TRIGGER issued_upin_append_only
    BEFORE UPDATE OR DELETE ON cadastre.issued_upin
    FOR EACH ROW EXECUTE FUNCTION cadastre.issued_upin_append_only();

-- Move the allocator above the highest standard UPIN ('PL' + 12 digits) in the ledger.
-- Never moves it backwards. Returns the next value the allocator will hand out.
CREATE OR REPLACE FUNCTION cadastre.sync_upin_sequence() RETURNS bigint
LANGUAGE plpgsql AS $$
DECLARE
    issued_max bigint;
    seq_last   bigint;
    seq_called boolean;
    seq_used   bigint;
BEGIN
    SELECT max(substring(upin FROM 3)::bigint) INTO issued_max
    FROM cadastre.issued_upin WHERE upin ~ '^PL[0-9]{12}$';

    SELECT last_value, is_called INTO seq_last, seq_called FROM cadastre.parcel_number_seq;
    seq_used := CASE WHEN seq_called THEN seq_last ELSE seq_last - 1 END;

    IF issued_max IS NOT NULL AND issued_max > seq_used THEN
        PERFORM setval('cadastre.parcel_number_seq', issued_max, true);
        RETURN issued_max + 1;
    END IF;
    RETURN seq_used + 1;
END $$;

-- Backfill from everything already in this database.
INSERT INTO cadastre.issued_upin (upin, source)
SELECT parcel_number, 'existing' FROM cadastre.parcel WHERE parcel_number IS NOT NULL
ON CONFLICT (upin) DO NOTHING;

INSERT INTO cadastre.issued_upin (upin, source)
SELECT DISTINCT ON (parcel_number) parcel_number, 'import:' || run_id || ':' || status
FROM import.parcel WHERE parcel_number IS NOT NULL
ORDER BY parcel_number, run_id
ON CONFLICT (upin) DO NOTHING;

INSERT INTO cadastre.issued_upin (upin, source)
SELECT DISTINCT ON (parcel_number) parcel_number, 'import-exception:' || run_id
FROM import.exception WHERE parcel_number IS NOT NULL AND btrim(parcel_number) <> ''
ORDER BY parcel_number, run_id
ON CONFLICT (upin) DO NOTHING;

SELECT cadastre.sync_upin_sequence();
