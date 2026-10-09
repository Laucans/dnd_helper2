--
-- PostgreSQL database dump
--


-- Dumped from database version 16.15 (Debian 16.15-1.pgdg13+2)
-- Dumped by pg_dump version 16.15 (Homebrew)

SET statement_timeout = 0;
SET lock_timeout = 0;
SET idle_in_transaction_session_timeout = 0;
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SELECT pg_catalog.set_config('search_path', '', false);
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;
SET row_security = off;

SET default_tablespace = '';

SET default_table_access_method = heap;

--
-- Name: campagne; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.campagne (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    nom text NOT NULL,
    "creeLe" timestamp with time zone DEFAULT now() NOT NULL,
    "archiveLe" timestamp with time zone,
    CONSTRAINT campagne_nom_nfc CHECK ((nom IS NFC NORMALIZED))
);


--
-- Name: campagne_active; Type: VIEW; Schema: public; Owner: -
--

CREATE VIEW public.campagne_active WITH (security_barrier='true') AS
 SELECT id,
    nom,
    "creeLe"
   FROM public.campagne
  WHERE ("archiveLe" IS NULL);


--
-- Name: dataguard_version; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.dataguard_version (
    singleton boolean DEFAULT true NOT NULL,
    version bigint DEFAULT 0 NOT NULL,
    CONSTRAINT dataguard_version_nonnegative CHECK ((version >= 0)),
    CONSTRAINT dataguard_version_singleton CHECK (singleton)
);


--
-- Name: data_version; Type: VIEW; Schema: public; Owner: -
--

CREATE VIEW public.data_version AS
 SELECT version AS "dataVersion"
   FROM public.dataguard_version;


--
-- Name: dataguard_enqueue_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.dataguard_enqueue_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: dataguard_hold; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.dataguard_hold (
    id uuid NOT NULL,
    partition text NOT NULL,
    author text NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    created_at timestamp with time zone NOT NULL
);


--
-- Name: dataguard_queue; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.dataguard_queue (
    command uuid NOT NULL,
    enqueue_seq bigint NOT NULL,
    data_capability text NOT NULL,
    author text NOT NULL,
    partition text NOT NULL,
    "position" bigint NOT NULL,
    scope text NOT NULL,
    mode text NOT NULL,
    touches text[] NOT NULL,
    effects text[] DEFAULT '{}'::text[] NOT NULL,
    operation text NOT NULL,
    payload text NOT NULL,
    payload_digest text NOT NULL,
    idempotency_key text,
    based_on text NOT NULL,
    projection jsonb,
    your_value jsonb,
    state text NOT NULL,
    confirmation jsonb,
    parked jsonb,
    parked_at timestamp with time zone,
    requeued_from uuid,
    messages jsonb DEFAULT '[]'::jsonb NOT NULL,
    warnings text[] DEFAULT '{}'::text[] NOT NULL,
    violations text[] DEFAULT '{}'::text[] NOT NULL,
    data_version bigint,
    impact_plan jsonb,
    enqueued_at timestamp with time zone NOT NULL,
    settled_at timestamp with time zone,
    CONSTRAINT dataguard_queue_mode CHECK ((mode = ANY (ARRAY['confirm_on_stale'::text, 'overwrite'::text, 'relative'::text]))),
    CONSTRAINT dataguard_queue_position_nonnegative CHECK (("position" >= 0)),
    CONSTRAINT dataguard_queue_settled CHECK (((state = ANY (ARRAY['applied'::text, 'rejected'::text, 'cancelled'::text, 'expired'::text])) = (settled_at IS NOT NULL))),
    CONSTRAINT dataguard_queue_state CHECK ((state = ANY (ARRAY['queued'::text, 'awaiting_confirmation'::text, 'confirmed'::text, 'awaiting_review'::text, 'parked'::text, 'applied'::text, 'rejected'::text, 'cancelled'::text, 'expired'::text]))),
    CONSTRAINT dataguard_queue_version CHECK (((state = 'applied'::text) = (data_version IS NOT NULL)))
);


--
-- Name: pj; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.pj (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    "campagneId" uuid NOT NULL,
    nom text NOT NULL,
    classe text NOT NULL,
    niveau integer NOT NULL,
    "creeLe" timestamp with time zone DEFAULT now() NOT NULL,
    "archiveLe" timestamp with time zone,
    CONSTRAINT pj_classe_nfc CHECK ((classe IS NFC NORMALIZED)),
    CONSTRAINT pj_nom_nfc CHECK ((nom IS NFC NORMALIZED))
);


--
-- Name: pj_actif; Type: VIEW; Schema: public; Owner: -
--

CREATE VIEW public.pj_actif WITH (security_barrier='true') AS
 SELECT p.id,
    p."campagneId",
    p.nom,
    p.classe,
    p.niveau,
    p."creeLe"
   FROM (public.pj p
     JOIN public.campagne c ON ((c.id = p."campagneId")))
  WHERE ((p."archiveLe" IS NULL) AND (c."archiveLe" IS NULL));


--
-- Name: campagne campagne_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.campagne
    ADD CONSTRAINT campagne_pkey PRIMARY KEY (id);


--
-- Name: dataguard_hold dataguard_hold_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.dataguard_hold
    ADD CONSTRAINT dataguard_hold_pkey PRIMARY KEY (id);


--
-- Name: dataguard_queue dataguard_queue_enqueue_seq_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.dataguard_queue
    ADD CONSTRAINT dataguard_queue_enqueue_seq_key UNIQUE (enqueue_seq);


--
-- Name: dataguard_queue dataguard_queue_partition_position; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.dataguard_queue
    ADD CONSTRAINT dataguard_queue_partition_position UNIQUE (partition, "position");


--
-- Name: dataguard_queue dataguard_queue_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.dataguard_queue
    ADD CONSTRAINT dataguard_queue_pkey PRIMARY KEY (command);


--
-- Name: dataguard_version dataguard_version_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.dataguard_version
    ADD CONSTRAINT dataguard_version_pkey PRIMARY KEY (singleton);


--
-- Name: pj pj_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.pj
    ADD CONSTRAINT pj_pkey PRIMARY KEY (id);


--
-- Name: dataguard_queue_effects; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX dataguard_queue_effects ON public.dataguard_queue USING gin (effects);


--
-- Name: dataguard_queue_idempotency; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX dataguard_queue_idempotency ON public.dataguard_queue USING btree (author, data_capability, idempotency_key) WHERE (idempotency_key IS NOT NULL);


--
-- Name: dataguard_queue_pending; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX dataguard_queue_pending ON public.dataguard_queue USING btree (state, enqueue_seq);


--
-- Name: pj_campagne_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX pj_campagne_idx ON public.pj USING btree ("campagneId");


--
-- Name: pj pj_campagne_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.pj
    ADD CONSTRAINT pj_campagne_fk FOREIGN KEY ("campagneId") REFERENCES public.campagne(id) ON UPDATE RESTRICT ON DELETE RESTRICT;


--
-- PostgreSQL database dump complete
--


