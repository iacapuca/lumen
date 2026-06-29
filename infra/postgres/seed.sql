-- Lumen — TPC-H sample data, self-contained (no dbgen required).
-- Real region/nation dimensions + generated customers/orders/lineitem with
-- spec-faithful column names and the canonical 1992–1998 date window.
-- Idempotent: drops and recreates. Yields ~5k orders / ~20k lineitems.

DROP TABLE IF EXISTS lineitem CASCADE;
DROP TABLE IF EXISTS orders   CASCADE;
DROP TABLE IF EXISTS customer CASCADE;
DROP TABLE IF EXISTS nation   CASCADE;
DROP TABLE IF EXISTS region   CASCADE;

CREATE TABLE region (
    r_regionkey INT PRIMARY KEY,
    r_name      CHAR(25),
    r_comment   VARCHAR(152)
);

CREATE TABLE nation (
    n_nationkey INT PRIMARY KEY,
    n_name      CHAR(25),
    n_regionkey INT REFERENCES region,
    n_comment   VARCHAR(152)
);

CREATE TABLE customer (
    c_custkey    INT PRIMARY KEY,
    c_name       VARCHAR(25),
    c_address    VARCHAR(40),
    c_nationkey  INT REFERENCES nation,
    c_phone      CHAR(15),
    c_acctbal    DECIMAL(15,2),
    c_mktsegment CHAR(10),
    c_comment    VARCHAR(117)
);

CREATE TABLE orders (
    o_orderkey      INT PRIMARY KEY,
    o_custkey       INT REFERENCES customer,
    o_orderstatus   CHAR(1),
    o_totalprice    DECIMAL(15,2),
    o_orderdate     DATE,
    o_orderpriority CHAR(15),
    o_clerk         CHAR(15),
    o_shippriority  INT,
    o_comment       VARCHAR(79)
);

CREATE TABLE lineitem (
    l_orderkey      INT REFERENCES orders,
    l_partkey       INT,
    l_suppkey       INT,
    l_linenumber    INT,
    l_quantity      DECIMAL(15,2),
    l_extendedprice DECIMAL(15,2),
    l_discount      DECIMAL(15,2),
    l_tax           DECIMAL(15,2),
    l_returnflag    CHAR(1),
    l_linestatus    CHAR(1),
    l_shipdate      DATE,
    l_commitdate    DATE,
    l_receiptdate   DATE,
    l_shipinstruct  CHAR(25),
    l_shipmode      CHAR(10),
    l_comment       VARCHAR(44),
    PRIMARY KEY (l_orderkey, l_linenumber)
);

-- 5 real regions
INSERT INTO region VALUES
 (0,'AFRICA',''),(1,'AMERICA',''),(2,'ASIA',''),(3,'EUROPE',''),(4,'MIDDLE EAST','');

-- 25 real nations
INSERT INTO nation VALUES
 (0,'ALGERIA',0,''),(1,'ARGENTINA',1,''),(2,'BRAZIL',1,''),(3,'CANADA',1,''),(4,'EGYPT',4,''),
 (5,'ETHIOPIA',0,''),(6,'FRANCE',3,''),(7,'GERMANY',3,''),(8,'INDIA',2,''),(9,'INDONESIA',2,''),
 (10,'IRAN',4,''),(11,'IRAQ',4,''),(12,'JAPAN',2,''),(13,'JORDAN',4,''),(14,'KENYA',0,''),
 (15,'MOROCCO',0,''),(16,'MOZAMBIQUE',0,''),(17,'PERU',1,''),(18,'CHINA',2,''),(19,'ROMANIA',3,''),
 (20,'SAUDI ARABIA',4,''),(21,'VIETNAM',2,''),(22,'RUSSIA',3,''),(23,'UNITED KINGDOM',3,''),(24,'UNITED STATES',1,'');

-- 1,500 customers
INSERT INTO customer
SELECT g,
       'Customer#'||lpad(g::text,9,'0'),
       md5(g::text),
       floor(random()*25)::int,
       lpad(floor(random()*25+10)::text,2,'0')||'-'||lpad(floor(random()*900+100)::text,3,'0')||'-'||lpad(floor(random()*900+100)::text,3,'0'),
       round((random()*10000-1000)::numeric,2),
       (array['BUILDING','AUTOMOBILE','MACHINERY','HOUSEHOLD','FURNITURE'])[floor(random()*5+1)],
       'seed'
FROM generate_series(1,1500) g;

-- 5,000 orders across 1992-01-01 .. 1998-12-31
INSERT INTO orders
SELECT g,
       floor(random()*1500+1)::int,
       (array['O','F','P'])[floor(random()*3+1)],
       round((random()*200000+1000)::numeric,2),
       date '1992-01-01' + floor(random()*2557)::int,
       (array['1-URGENT','2-HIGH','3-MEDIUM','4-NOT SPECIFIED','5-LOW'])[floor(random()*5+1)],
       'Clerk#'||lpad(floor(random()*1000+1)::text,9,'0'),
       0,'seed'
FROM generate_series(1,5000) g;

-- 1–7 lineitems per order (~20k rows). The line count is correlated to
-- o_orderkey so Postgres evaluates it per-order (an uncorrelated random() in
-- the LATERAL bound gets constant-folded to a single value for the whole query).
INSERT INTO lineitem
SELECT o.o_orderkey,
       floor(random()*2000+1)::int,
       floor(random()*100+1)::int,
       li.n,
       round((random()*49+1)::numeric,2),                -- l_quantity
       round((random()*40000+900)::numeric,2),           -- l_extendedprice
       round((floor(random()*11)/100.0)::numeric,2),     -- l_discount 0.00–0.10
       round((floor(random()*9)/100.0)::numeric,2),      -- l_tax
       (array['A','N','R'])[floor(random()*3+1)],
       (array['O','F'])[floor(random()*2+1)],
       o.o_orderdate + floor(random()*120)::int,
       o.o_orderdate + floor(random()*90)::int,
       o.o_orderdate + floor(random()*120)::int,
       'DELIVER IN PERSON',
       (array['TRUCK','MAIL','SHIP','RAIL','AIR','REG AIR','FOB'])[floor(random()*7+1)],
       'seed'
FROM orders o
CROSS JOIN LATERAL generate_series(1, 1 + (o.o_orderkey % 7)) AS li(n);

CREATE INDEX IF NOT EXISTS orders_orderdate_idx ON orders (o_orderdate);
CREATE INDEX IF NOT EXISTS lineitem_orderkey_idx ON lineitem (l_orderkey);
