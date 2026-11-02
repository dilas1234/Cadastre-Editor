-- Fix: the local UPIN allocator was being driven to ~9.8e11 by a handful of mis-imported
-- "structured" UPINs (numeric value >= 1e9, e.g. PL983205519013), so newly posted parcels were
-- numbered PL983205519016 instead of continuing the real local series (…PL000000023893).
--
-- Locally allocated UPINs are small and (near) contiguous: `PL` + a 12-digit value that counts up
-- from 1. A few imported records carry a UPIN encoding something else entirely (values in the tens
-- of billions); those must be recorded in the ledger for collision-avoidance, but must NOT drive
-- the local sequential counter. We treat any UPIN whose value is >= 1e9 as externally structured.

-- Only standard, locally-ranged UPINs (value < 1e9) move the allocator. Never moves it backwards.
CREATE OR REPLACE FUNCTION cadastre.sync_upin_sequence() RETURNS bigint
LANGUAGE plpgsql AS $$
DECLARE
    issued_max bigint;
    seq_last   bigint;
    seq_called boolean;
    seq_used   bigint;
BEGIN
    SELECT max(substring(upin FROM 3)::bigint) INTO issued_max
    FROM cadastre.issued_upin
    WHERE upin ~ '^PL[0-9]{12}$' AND substring(upin FROM 3)::bigint < 1000000000;

    SELECT last_value, is_called INTO seq_last, seq_called FROM cadastre.parcel_number_seq;
    seq_used := CASE WHEN seq_called THEN seq_last ELSE seq_last - 1 END;

    IF issued_max IS NOT NULL AND issued_max > seq_used THEN
        PERFORM setval('cadastre.parcel_number_seq', issued_max, true);
        RETURN issued_max + 1;
    END IF;
    RETURN seq_used + 1;
END $$;

-- One-time correction: the sequence was already poisoned to a structured value, and
-- sync_upin_sequence() only ever moves forward, so reset it down to the true local sequential max.
SELECT setval(
    'cadastre.parcel_number_seq',
    GREATEST(1, COALESCE((
        SELECT max(substring(upin FROM 3)::bigint)
        FROM cadastre.issued_upin
        WHERE upin ~ '^PL[0-9]{12}$' AND substring(upin FROM 3)::bigint < 1000000000
    ), 0)),
    true
);
