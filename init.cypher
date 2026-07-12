CREATE NODE TABLE Person(name STRING, age INT64, score DOUBLE, active BOOL, PRIMARY KEY(name));
COPY Person FROM 'dataset/tinysnb/person.csv' (HEADER true);
