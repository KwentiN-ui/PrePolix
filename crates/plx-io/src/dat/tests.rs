//! Excerpts of `.dat` files CalculiX 2.21 wrote for a cantilever and two blocks in contact.

use super::*;

const STATIC: &str = "
                        S T E P       1


                                INCREMENT     1


 displacements (vx,vy,vz) for set INTERNAL_SELECTION-1_NH_OUTPUT-1 and time  0.5000000E+00

        41 -1.422638E-02  3.592771E-05 -1.902165E-01
        62 -1.422384E-02  1.545322E-05 -1.901480E-01

 total force (fx,fy,fz) for set FIX and time  0.5000000E+00

       -1.435438E-10 -1.598579E-10  5.000000E+01

 stresses (elem, integ.pnt.,sxx,syy,szz,sxy,sxz,syz) for set EALL and time  0.5000000E+00

         1   1  1.000000E+02  0.000000E+00  0.000000E+00  0.000000E+00  0.000000E+00  0.000000E+00
         1   2 -4.443359E+01  6.381342E-01  1.795932E+00  6.517116E-01  5.780083E-01  4.543917E-01

 internal energy density (elem, integ.pnt.,energy) for set EALL and time  0.5000000E+00

         1   1  4.736790E-03

 internal energy (element, energy) for set EALL and time  0.5000000E+00

         1  9.342906E-09

 total volume for set EALL and time  0.5000000E+00

        1.000000E+04

                                INCREMENT     2


 displacements (vx,vy,vz) for set INTERNAL_SELECTION-1_NH_OUTPUT-1 and time  0.1000000E+01

        41 -2.845276E-02  7.185542E-05 -3.804330E-01
        62 -2.844768E-02  3.090644E-05 -3.802960E-01

 total force (fx,fy,fz) for set FIX and time  0.1000000E+01

       -2.870876E-10 -3.197158E-10  1.000000E+02
";

#[test]
fn nodal_values_and_totals_over_increments() {
    let import = parse_dat(STATIC);
    assert!(import.warnings.is_empty(), "{:?}", import.warnings);
    let names: Vec<&str> = import.sets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["NH_OUTPUT-1", "FIX", "EALL"]);
    let set = &import.sets[0];
    assert_eq!(set.rows, [(1, 1, 0.5), (1, 2, 1.0)]);
    let u3 = set.field("DISPLACEMENTS").unwrap().component("U3").unwrap();
    let names: Vec<&str> = u3.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["41", "62"]);
    assert_eq!(u3.entries[0].values, [-1.902165E-01, -3.804330E-01]);
    // A sum has one entry per component.
    let total = import.sets[1].field("TOTAL_FORCE").unwrap();
    let rf3 = total.component("RF3").unwrap();
    assert_eq!(rf3.entries.len(), 1);
    assert_eq!(rf3.entries[0].name, "RF3");
    assert_eq!(rf3.entries[0].values, [50.0, 100.0]);
}

#[test]
fn element_values_by_integration_point_with_von_mises() {
    let import = parse_dat(STATIC);
    let set = import.sets.iter().find(|s| s.name == "EALL").unwrap();
    assert_eq!(set.rows, [(1, 1, 0.5)]);
    let stresses = set.field("STRESSES").unwrap();
    let names: Vec<&str> = stresses
        .components
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "MISES",
            "TRESCA",
            "S11",
            "S22",
            "S33",
            "S12",
            "S13",
            "S23",
            "SGN_MAX_ABS_PRI",
            "PRINCIPAL_MAX",
            "PRINCIPAL_MID",
            "PRINCIPAL_MIN"
        ]
    );
    let mises = stresses.component("MISES").unwrap();
    assert_eq!(mises.entries[0].name, "1_1");
    assert!((mises.entries[0].values[0] - 100.0).abs() < 1e-9);
    let energy = set.field("INTERNAL_ENERGY_DENSITY").unwrap();
    assert_eq!(energy.components[0].name, "ENER");
    let element = set
        .field("INTERNAL_ENERGY")
        .unwrap()
        .component("ELSE")
        .unwrap();
    assert_eq!(element.entries[0].name, "1");
    let volume = set.field("TOTAL_VOLUME").unwrap().component("VOL").unwrap();
    assert_eq!(volume.entries[0].values, [1.0E4]);
}

const CONTACT: &str = "
                        S T E P       1


                                INCREMENT     1


 relative contact displacement (slave element+face,normal,tang1,tang2) for all contact elements and time 0.1000000E+01

        65          1 -1.000000E-04  0.000000E+00  0.000000E+00
        65          1 -2.000000E-04  0.000000E+00  0.000000E+00
        66          1 -1.056368E-04  0.000000E+00  0.000000E+00

 total number of contact elements for time  0.1000000E+01

              448


 statistics for slave set INTERNAL_SELECTION-2_CONTACT_PAIR-1_SLAVE, master set INTERNAL_SELECTION-1_CONTACT_PAIR-1_MASTER and time  0.1000000E+01

   total surface force (fx,fy,fz) and moment about the origin (mx,my,mz)

    5.176189E-13 -3.853183E-13  1.081277E+04  5.406386E+04 -5.406386E+04 -5.710567E-12

   center of gravity and mean normal

    5.000000E+00  5.000000E+00  9.994946E+00 -4.815283E-17  3.711645E-17 -1.000000E+00

   moment about the center of gravity(mx,my,mz)

    1.515616E-07 -3.863533E-09 -1.195881E-12

   area,  normal force (+ = tension) and shear force (size)

    1.000275E+02 -1.081277E+04  1.498847E-09
";

#[test]
fn contact_values_are_averaged_per_face_and_statistics_named_by_pair() {
    let import = parse_dat(CONTACT);
    assert!(import.warnings.is_empty(), "{:?}", import.warnings);
    let all = import
        .sets
        .iter()
        .find(|s| s.name == ALL_CONTACT_ELEMENTS)
        .unwrap();
    let field = all.field("RELATIVE_CONTACT_DISPLACEMENT").unwrap();
    let normal = field.component("NORMAL").unwrap();
    assert_eq!(normal.entries.len(), 2);
    assert_eq!(normal.entries[0].name, "65_1");
    assert!((normal.entries[0].values[0] + 1.5e-4).abs() < 1e-12);
    let count = all.field("TOTAL_NUMBER_OF_CONTACT_ELEMENTS").unwrap();
    assert_eq!(count.components[0].entries[0].values, [448.0]);
    let pair = import
        .sets
        .iter()
        .find(|s| s.name == "CONTACT_PAIR-1")
        .unwrap();
    let force = pair.field("TOTAL_SURFACE_FORCE").unwrap();
    assert_eq!(
        force.component("FZ").unwrap().entries[0].values,
        [1.081277E+04]
    );
    let loads = pair.field("SURFACE_LOADS").unwrap();
    assert_eq!(
        loads.component("NORMAL_FORCE").unwrap().entries[0].values,
        [-1.081277E+04]
    );
    assert_eq!(
        pair.field("SURFACE_AREA").unwrap().components[0].entries[0].values,
        [1.000275E+02]
    );
}

const FREQUENCY: &str = "
                        S T E P       1


     E I G E N V A L U E   O U T P U T

 MODE NO    EIGENVALUE                       FREQUENCY   
                                     REAL PART            IMAGINARY PART
                           (RAD/TIME)      (CYCLES/TIME     (RAD/TIME)

      1   0.2747655E+08   0.5241808E+04   0.8342596E+03   0.0000000E+00
      2   0.9890009E+09   0.3144838E+05   0.5005166E+04   0.0000000E+00

     P A R T I C I P A T I O N   F A C T O R S

MODE NO.   X-COMPONENT     Y-COMPONENT     Z-COMPONENT     X-ROTATION      Y-ROTATION      Z-ROTATION

      1  -0.3283311E-16  -0.5293366E-03  -0.6907440E-02  -0.3189052E-01   0.5030764E+00  -0.3855216E-01
      2   0.4999019E-16   0.4219778E-03   0.3862376E-02   0.1720199E-01  -0.8058018E-01   0.8803661E-02

     E F F E C T I V E   M O D A L   M A S S

MODE NO.   X-COMPONENT     Y-COMPONENT     Z-COMPONENT     X-ROTATION      Y-ROTATION      Z-ROTATION

      1   0.1078013E-32   0.2801973E-06   0.4771273E-04   0.1017005E-02   0.2530858E+00   0.1486269E-02
      2   0.2499019E-32   0.1780653E-06   0.1491795E-04   0.2959085E-03   0.6493166E-02   0.7750445E-04
TOTAL     0.1103095E-31   0.6308894E-04   0.6308894E-04   0.3154447E-02   0.2611428E+00   0.2611428E+00

     T O T A L   E F F E C T I V E   M A S S

MODE NO.   X-COMPONENT     Y-COMPONENT     Z-COMPONENT     X-ROTATION      Y-ROTATION      Z-ROTATION

          0.7762778E-04   0.7762778E-04   0.7762778E-04   0.5175185E-02   0.2642543E+00   0.2642543E+00


                    E I G E N V A L U E    N U M B E R     1


 displacements (vx,vy,vz) for set TIP and time  0.1000000E+01

        41 -1.653821E+01 -1.719468E+01 -2.243813E+02

                    E I G E N V A L U E    N U M B E R     2


 displacements (vx,vy,vz) for set TIP and time  0.1000000E+01

        41 -1.418389E+01 -2.243813E+02  1.719527E+01
";

#[test]
fn frequency_steps_have_a_row_per_mode() {
    let import = parse_dat(FREQUENCY);
    assert!(import.warnings.is_empty(), "{:?}", import.warnings);
    let step = import.sets.iter().find(|s| s.name == "STEP_1").unwrap();
    assert_eq!(step.rows, [(1, 1, 834.2596), (1, 2, 5005.166)]);
    let frequency = step.field("EIGENVALUE_OUTPUT").unwrap();
    let names: Vec<&str> = frequency
        .components
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(names, ["EIGENVALUE", "OMEGA", "FREQUENCY", "FREQUENCY_IM"]);
    let mass = step.field("EFFECTIVE_MODAL_MASS").unwrap();
    assert_eq!(
        mass.component("Z_COMPONENT").unwrap().entries[0].values,
        [0.4771273E-04, 0.1491795E-04]
    );
    let tip = import.sets.iter().find(|s| s.name == "TIP").unwrap();
    assert_eq!(tip.rows, step.rows);
    let u1 = tip.field("DISPLACEMENTS").unwrap().component("U1").unwrap();
    assert_eq!(u1.entries[0].values, [-1.653821E+01, -1.418389E+01]);
}

/// The `.dat` file of the buckle step of the cantilever, from CalculiX 2.21.
const BUCKLING: &str = "

                        S T E P       1


     B U C K L I N G   F A C T O R   O U T P U T

 MODE NO       BUCKLING
                FACTOR

      1   0.2051835E+04
      2   0.2051835E+04

";

#[test]
fn buckle_steps_list_the_factor_of_each_mode() {
    let import = parse_dat(BUCKLING);
    assert!(import.warnings.is_empty(), "{:?}", import.warnings);
    let step = import.sets.iter().find(|s| s.name == "STEP_1").unwrap();
    assert_eq!(step.rows, [(1, 1, 2051.835), (1, 2, 2051.835)]);
    let factors = step.field("BUCKLING_FACTOR_OUTPUT").unwrap();
    let factor = factors.component("BUCKLING_FACTOR").unwrap();
    assert_eq!(factor.entries[0].values, [2051.835, 2051.835]);
}

#[test]
fn numbers_without_exponent_letter() {
    assert_eq!(parse_number("1.5-282"), 1.5e-282);
    assert_eq!(parse_number("-2.5+300"), -2.5e300);
    assert!(parse_number("x").is_nan());
}

#[test]
fn unknown_blocks_are_reported_once() {
    let text = " foo bar (a,b) for set X and time 1.0\n\n 1 2\n\n foo bar (a,b) for set X and time 2.0\n\n 1 2\n";
    let import = parse_dat(text);
    assert!(import.sets.is_empty());
    assert_eq!(import.warnings.len(), 1);
    assert!(import.warnings[0].contains("foo bar"));
}

const COMPLEX_FREQUENCY: &str = "
                        S T E P       3


     E I G E N V A L U E   O U T P U T

 MODE NO                     FREQUENCY                   
                      REAL PART         IMAGINARY PART
             (RAD/TIME)   (CYCLES/TIME)   (RAD/TIME)

      1   0.4602752E+04   0.7325508E+03   0.4018004E-11
      2   0.6595139E+04   0.1049649E+04   0.5608028E-12

P A R T I C I P A T I O N   F A C T O R S   F O R   M O D E       1

  0.6746E-01  0.6754E-01  0.7071E+00  0.7071E+00  0.1114E-13  0.5820E-14

     E I G E N M O D E   T U R N I N G   D I R E C T I O N

    Axis reference direction:  0.1000E+01  0.0000E+00  0.0000E+00

 MODE NO     TURNING DIRECTION (F=FORWARD,B=BACKWARD)

      1          F
      2          B
";

#[test]
fn complex_frequency_steps_have_real_and_imaginary_parts_and_a_whirl() {
    let import = parse_dat(COMPLEX_FREQUENCY);
    assert!(import.warnings.is_empty(), "{:?}", import.warnings);
    let set = &import.sets[0];
    assert_eq!(set.name, "STEP_3");
    let output = set.field("EIGENVALUE_OUTPUT").unwrap();
    let component = |name: &str| &output.component(name).unwrap().entries[0].values;
    assert_eq!(component("FREQUENCY"), &[732.5508, 1049.649]);
    assert_eq!(component("OMEGA"), &[4602.752, 6595.139]);
    assert_eq!(component("OMEGA_IM"), &[4.018004e-12, 5.608028e-13]);
    assert_eq!(component("TURNING_DIRECTION"), &[1.0, -1.0]);
    assert_eq!(
        set.rows.iter().map(|r| r.2).collect::<Vec<_>>(),
        [732.5508, 1049.649]
    );
}
